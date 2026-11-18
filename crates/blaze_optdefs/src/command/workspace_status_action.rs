//! Transcription of `com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:70 WorkspaceStatusAction.Options
options! {
    /// Options controlling the workspace status command.
    pub struct WorkspaceStatusActionOptions {
        #[option(
            name = "embed_label",
            default_value = "",
            converter = OneLineStringConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "Embed source control revision or release label in binary",
        )]
        pub embed_label: String,

        #[option(
            name = "workspace_status_command",
            default_value = "",
            converter = PathFragmentConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            value_help = "<path>",
            help = "A command invoked at the beginning of the build to provide status\ninformation about the workspace in the form of key/value pairs.\nSee the User's Manual for the full specification. Also see\n[`tools/buildstamp/get_workspace_status`][wksp-stat] for an example.\n\n[wksp-stat]: https://github.com/bazelbuild/bazel/blob/master/tools/buildstamp/get_workspace_status\n",
        )]
        pub workspace_status_command: Foreign<PathFragment>,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const WORKSPACE_STATUS_ACTION_OPTIONS_OPTION_NAMES: &[&str] =
        &["embed_label", "workspace_status_command"];

    #[test]
    fn workspace_status_action_options_option_names() {
        super::super::test_util::check_option_names(
            WorkspaceStatusActionOptions::class(),
            WORKSPACE_STATUS_ACTION_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn workspace_status_action_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<WorkspaceStatusActionOptions>(
            SKIP_ROUND_TRIP,
        );
    }
}

// ---- hand-written below this line (preserved by the generator) ----

// Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:220 WorkspaceStatusAction.OneLineStringConverter
/// Converter for options that must not contain a newline.
#[derive(Clone, Copy, Debug, Default)]
pub struct OneLineStringConverter;

impl Converter for OneLineStringConverter {
    // Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:223 WorkspaceStatusAction.OneLineStringConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        if input.contains('\n') {
            return Err(OptionsParsingException::new(
                "Value must not contain multiple lines",
            ));
        }
        Ok(Some(Value::new(input.to_string())))
    }
    fn get_type_description(&self) -> String {
        "a one-line string".into()
    }
}

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::test_util::{parse, parse_err};
    use super::*;

    #[test]
    fn embed_label() {
        let o = parse::<WorkspaceStatusActionOptions>(&["--embed_label=v1"]);
        assert_eq!(o.embed_label, "v1");
        assert_eq!(
            parse_err::<WorkspaceStatusActionOptions>(&["--embed_label=a\nb"]),
            "While parsing option --embed_label=a\nb: Value must not contain multiple lines"
        );
    }
}
