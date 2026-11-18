//! Port of `buildeventservice/BuildEventServiceOptions.java` (the `OptionsBase` subclass and what it uses).

use std::time::Duration;

use blaze_options::{
    AssignmentConverter, CommaSeparatedOptionListConverter, DurationConverter, enum_converter,
    option_enum, options,
};

// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceOptions.java:29 BuildEventServiceOptions
options! {
    /// Options used by `BuildEventServiceModule`.
    pub struct BuildEventServiceOptions {
        #[option(
            name = "bes_backend",
            default_value = "",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Specifies the build event service (BES) backend endpoint in the form\n",
                "`[SCHEME://]HOST[:PORT]`. The default is to disable BES uploads. Supported schemes\n",
                "are `grpc` and `grpcs` (grpc with TLS enabled). If no scheme is provided, Bazel\n",
                "assumes `grpcs`.\n",
            ),
        )]
        pub bes_backend: String,

        #[option(
            name = "bes_timeout",
            default_value = "0s",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Specifies how long bazel should wait for the BES/BEP upload to complete after the\n",
                "build and tests have finished. A valid timeout is a natural number followed by a\n",
                "unit: Days (d), hours (h), minutes (m), seconds (s), and milliseconds (ms). The\n",
                "default value is `0` which means that there is no timeout.\n",
            ),
        )]
        pub bes_timeout: Duration,

        #[option(
            name = "bes_header",
            converter = AssignmentConverter,
            default_value = "null",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Specify a header in `NAME=VALUE` form that will be included in BES requests.\n",
                "Multiple headers can be passed by specifying the flag multiple times. Multiple\n",
                "values for the same name will be converted to a comma-separated list.\n",
            ),
            allow_multiple = true,
        )]
        pub bes_headers: Vec<(String, String)>,

        #[option(
            name = "bes_lifecycle_events",
            default_value = "true",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "Specifies whether to publish BES lifecycle events. (defaults to 'true').",
        )]
        pub bes_lifecycle_events: bool,

        #[option(
            name = "bes_instance_name",
            old_name = "project_id",
            default_value = "null",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Specifies the instance name under which the BES will persist uploaded BEP. Defaults ",
                "to null.",
            ),
        )]
        pub instance_name: Option<String>,

        #[option(
            name = "bes_keywords",
            default_value = "null",
            converter = CommaSeparatedOptionListConverter,
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            allow_multiple = true,
            help = concat!(
                "Specifies a list of notification keywords to be added the default set of keywords\n",
                "published to BES (`command_name={command_name}`, `protocol_name=BEP`).\n",
                "Defaults to none.\n",
            ),
        )]
        pub bes_keywords: Vec<String>,

        #[option(
            name = "bes_system_keywords",
            default_value = "null",
            converter = CommaSeparatedOptionListConverter,
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            allow_multiple = true,
            help = concat!(
                "Specifies a list of notification keywords to be included directly, without the\n",
                "`user_keyword=` prefix included for keywords supplied via `--bes_keywords`.\n",
                "Intended for Build service operators that set `--bes_lifecycle_events=false` and\n",
                "include keywords when calling `PublishLifecycleEvent`. Build service operators\n",
                "using this flag should prevent users from overriding the flag value.\n",
            ),
        )]
        pub bes_system_keywords: Vec<String>,

        #[option(
            name = "bes_outerr_buffer_size",
            default_value = "10240",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Specifies the maximal size of stdout or stderr to be buffered in BEP, before it is\n",
                "reported as a progress event. Individual writes are still reported in a single\n",
                "event, even if larger than the specified value up to `--bes_outerr_chunk_size`.\n",
            ),
        )]
        pub bes_outerr_buffer_size: i32,

        #[option(
            name = "bes_outerr_chunk_size",
            default_value = "1048576",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "Specifies the maximal size of stdout or stderr to be sent to BEP in a single message.",
        )]
        pub bes_outerr_chunk_size: i32,

        #[option(
            name = "bes_results_url",
            default_value = "",
            documentation_category = LOGGING,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "Specifies the base URL where a user can view the information streamed to the BES",
                " backend. Bazel will output the URL appended by the invocation id to the",
                " terminal.",
            ),
        )]
        pub bes_results_url: String,

        #[option(
            name = "bes_upload_mode",
            default_value = "wait_for_upload_complete",
            converter = BesUploadModeConverter,
            documentation_category = LOGGING,
            effect_tags = [EAGERNESS_TO_EXIT],
            help = concat!(
                "Specifies whether the Build Event Service upload should block the build completion or\n",
                "should end the invocation immediately and finish the upload in the\n",
                "background.\n",
                "- `wait_for_upload_complete`: blocks at the end of the current invocation until\n",
                "  all events (including lifecycle events if applicable) are uploaded and\n",
                "  acknowledged by the backend.\n",
                "- `nowait_for_upload_complete`: blocks at the beginning of the next invocation\n",
                "  until all events (including lifecycle events if applicable) are uploaded and\n",
                "  acknowledged by the backend.\n",
                "- `fully_async`: blocks at the beginning of the next invocation until all events\n",
                "  are uploaded but does not wait for acknowledgements. Events may be lost in case\n",
                "  of (transient) failures and backends may report streams as incomplete in this\n",
                "  mode. There is no guarantee that `FinishInvocationAttempt` or `FinishBuild`\n",
                "  lifecycle events are sent.\n",
            ),
        )]
        pub bes_upload_mode: BesUploadMode,

        #[option(
            name = "bes_proxy",
            default_value = "null",
            documentation_category = LOGGING,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Connect to the Build Event Service through a proxy. Currently this flag can only be\n",
                "used to configure a Unix domain socket (`unix:/path/to/socket`).\n",
            ),
        )]
        pub bes_proxy: Option<String>,

        #[option(
            name = "bes_check_preceding_lifecycle_events",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Sets the field `check_preceding_lifecycle_events_present` on\n",
                "`PublishBuildToolEventStreamRequest` which tells BES to check whether it previously\n",
                "received `InvocationAttemptStarted` and `BuildEnqueued` events matching the current\n",
                "tool event.\n",
            ),
        )]
        pub bes_check_preceding_lifecycle_events: bool,

        #[option(
            name = "bes_oom_finish_upload_timeout",
            default_value = "10m",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            converter = DurationConverter,
            help = concat!(
                "Specifies how long bazel should wait for the BES/BEP upload to complete while OOMing. ",
                "This flag ensures termination when the JVM is severely GC thrashing and cannot ",
                "make progress on any user thread.",
            ),
        )]
        pub bes_oom_finish_upload_timeout: Duration,
    }
}

// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceOptions.java:223 BuildEventServiceOptions.BesUploadMode
option_enum! {
    /// Determines the mode that will be used to upload data to the Build Event Service.
    pub enum BesUploadMode {
        // Block at the end of the build waiting for the upload to complete
        WAIT_FOR_UPLOAD_COMPLETE,
        // Block at the beginning of the next build waiting for upload completion
        NOWAIT_FOR_UPLOAD_COMPLETE,
        // Block at the beginning of the next build waiting for the client to finish uploading the
        // data, but possibly not blocking on the server acknowledgement.
        FULLY_ASYNC,
    }
}

// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceOptions.java:236 BuildEventServiceOptions.BesUploadModeConverter
enum_converter!(
    pub BesUploadModeConverter,
    BesUploadMode,
    "Mode for uploading to the Build Event Service"
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::test_support::{check_options_class, parse};

    #[test]
    fn options_class_matches_java() {
        check_options_class::<BuildEventServiceOptions>(&[
            "bes_backend",
            "bes_timeout",
            "bes_header",
            "bes_lifecycle_events",
            "bes_instance_name",
            "bes_keywords",
            "bes_system_keywords",
            "bes_outerr_buffer_size",
            "bes_outerr_chunk_size",
            "bes_results_url",
            "bes_upload_mode",
            "bes_proxy",
            "bes_check_preceding_lifecycle_events",
            "bes_oom_finish_upload_timeout",
        ]);
    }

    #[test]
    fn parses_flags() {
        let o = parse::<BuildEventServiceOptions>(&[
            "--bes_header=a=b",
            "--bes_header=c=d",
            "--bes_keywords=x,y",
            "--bes_keywords=z",
            "--bes_upload_mode=fully_async",
            "--bes_timeout=10s",
        ])
        .unwrap();
        assert_eq!(
            o.bes_headers,
            vec![("a".into(), "b".into()), ("c".into(), "d".into())]
        );
        assert_eq!(o.bes_keywords, vec!["x", "y", "z"]);
        assert_eq!(o.bes_upload_mode, BesUploadMode::FULLY_ASYNC);
        assert_eq!(o.bes_timeout, Duration::from_secs(10));
        assert_eq!(
            parse::<BuildEventServiceOptions>(&["--bes_upload_mode=bogus"])
                .unwrap_err()
                .get_message(),
            "While parsing option --bes_upload_mode=bogus: Not a valid Mode for uploading to the \
             Build Event Service: 'bogus' (should be wait_for_upload_complete, \
             nowait_for_upload_complete or fully_async)"
        );
    }
}
