//! Port of `buildeventstream/transports/BuildEventStreamOptions.java` (the `OptionsBase` subclass and what it uses).

use blaze_options::options;

use super::build_event_service_options::{BesUploadMode, BesUploadModeConverter};

// Java: com/google/devtools/build/lib/buildeventstream/transports/BuildEventStreamOptions.java:26 BuildEventStreamOptions
options! {
    /// Options used to configure BuildEventStreamer and its BuildEventTransports.
    pub struct BuildEventStreamOptions {
        #[option(
            name = "build_event_text_file",
            default_value = "",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "If non-empty, write a textual representation of the build event protocol to that file",
        )]
        pub build_event_text_file: String,

        #[option(
            name = "keep_backend_build_event_connections_alive",
            default_value = "true",
            metadata_tags = [HIDDEN],
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = "If enabled, keep connections to build event backend connections alive across builds.",
        )]
        pub keep_backend_connections: bool,

        #[option(
            name = "build_event_binary_file",
            default_value = "",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "If non-empty, write a varint delimited binary representation of representation of the\n",
                "build event protocol to that file. This option implies\n",
                "`--bes_upload_mode=wait_for_upload_complete`.\n",
            ),
        )]
        pub build_event_binary_file: String,

        #[option(
            name = "build_event_json_file",
            default_value = "",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "If non-empty, write a JSON serialisation of the build event protocol to that file.\n",
                "This option implies `--bes_upload_mode=wait_for_upload_complete`.\n",
            ),
        )]
        pub build_event_json_file: String,

        #[option(
            name = "build_event_text_file_upload_mode",
            default_value = "wait_for_upload_complete",
            converter = BesUploadModeConverter,
            documentation_category = LOGGING,
            effect_tags = [EAGERNESS_TO_EXIT],
            help = concat!(
                "Specifies whether the Build Event Service upload for `--build_event_text_file` should\n",
                "block the build completion or should end the invocation immediately and finish\n",
                "the upload in the background. Either `wait_for_upload_complete` (default),\n",
                "`nowait_for_upload_complete`, or `fully_async`.\n",
            ),
        )]
        pub build_event_text_file_upload_mode: BesUploadMode,

        #[option(
            name = "build_event_binary_file_upload_mode",
            default_value = "wait_for_upload_complete",
            converter = BesUploadModeConverter,
            documentation_category = LOGGING,
            effect_tags = [EAGERNESS_TO_EXIT],
            help = concat!(
                "Specifies whether the Build Event Service upload for `--build_event_binary_file` should\n",
                "block the build completion or should end the invocation immediately and finish\n",
                "the upload in the background. Either `wait_for_upload_complete` (default),\n",
                "`nowait_for_upload_complete`, or `fully_async`.\n",
            ),
        )]
        pub build_event_binary_file_upload_mode: BesUploadMode,

        #[option(
            name = "build_event_json_file_upload_mode",
            default_value = "wait_for_upload_complete",
            converter = BesUploadModeConverter,
            documentation_category = LOGGING,
            effect_tags = [EAGERNESS_TO_EXIT],
            help = concat!(
                "Specifies whether the Build Event Service upload for `--build_event_json_file` should\n",
                "block the build completion or should end the invocation immediately and finish\n",
                "the upload in the background. Either `wait_for_upload_complete` (default),\n",
                "`nowait_for_upload_complete`, or `fully_async`.\n",
            ),
        )]
        pub build_event_json_file_upload_mode: BesUploadMode,

        #[option(
            name = "build_event_text_file_path_conversion",
            old_name = "experimental_build_event_text_file_path_conversion",
            default_value = "true",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Convert paths in the text file representation of the build event protocol to more\n",
                "globally valid URIs whenever possible; if disabled, the `file://` uri scheme will\n",
                "always be used\n",
            ),
        )]
        pub build_event_text_file_path_conversion: bool,

        #[option(
            name = "build_event_binary_file_path_conversion",
            old_name = "experimental_build_event_binary_file_path_conversion",
            default_value = "true",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Convert paths in the binary file representation of the build event protocol to more\n",
                "globally valid URIs whenever possible; if disabled, the `file://` uri scheme will\n",
                "always be used\n",
            ),
        )]
        pub build_event_binary_file_path_conversion: bool,

        #[option(
            name = "build_event_json_file_path_conversion",
            old_name = "experimental_build_event_json_file_path_conversion",
            default_value = "true",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Convert paths in the json file representation of the build event protocol to more\n",
                "globally valid URIs whenever possible; if disabled, the `file://` uri scheme will\n",
                "always be used\n",
            ),
        )]
        pub build_event_json_file_path_conversion: bool,

        #[option(
            name = "build_event_publish_all_actions",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "Whether all actions should be published.",
        )]
        pub publish_all_actions: bool,

        #[option(
            name = "build_event_max_named_set_of_file_entries",
            default_value = "5000",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "The maximum number of entries for a single `named_set_of_files` event; values smaller\n",
                "than 2 are ignored and no event splitting is performed. This is intended for\n",
                "limiting the maximum event size in the build event protocol, although it does not\n",
                "directly control event size. The total event size is a function of the structure\n",
                "of the set as well as the file and uri lengths, which may in turn depend on the\n",
                "hash function.\n",
            ),
        )]
        pub max_named_set_entries: i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::test_support::{check_options_class, parse};

    #[test]
    fn options_class_matches_java() {
        check_options_class::<BuildEventStreamOptions>(&[
            "build_event_text_file",
            "keep_backend_build_event_connections_alive",
            "build_event_binary_file",
            "build_event_json_file",
            "build_event_text_file_upload_mode",
            "build_event_binary_file_upload_mode",
            "build_event_json_file_upload_mode",
            "build_event_text_file_path_conversion",
            "build_event_binary_file_path_conversion",
            "build_event_json_file_path_conversion",
            "build_event_publish_all_actions",
            "build_event_max_named_set_of_file_entries",
        ]);
    }

    #[test]
    fn parses_flags() {
        let o = parse::<BuildEventStreamOptions>(&[
            "--build_event_json_file=/tmp/bep.json",
            "--build_event_json_file_upload_mode=nowait_for_upload_complete",
            "--experimental_build_event_json_file_path_conversion=false",
        ])
        .unwrap();
        assert_eq!(o.build_event_json_file, "/tmp/bep.json");
        assert_eq!(
            o.build_event_json_file_upload_mode,
            BesUploadMode::NOWAIT_FOR_UPLOAD_COMPLETE
        );
        assert!(!o.build_event_json_file_path_conversion);
    }
}
