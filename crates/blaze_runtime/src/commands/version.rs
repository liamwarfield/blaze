//! `blaze version`.

use blaze_events::{Event, EventHandler};
use blaze_optdefs::command::version_command::VersionCommandVersionOptions;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::options_result::OptionsParsingResult;
use crate::version_info::BlazeVersionInfo;

/// The 'blaze version' command, which informs users about the blaze version information.
// Java: com/google/devtools/build/lib/runtime/commands/VersionCommand.java:46 VersionCommand
pub struct VersionCommand;

impl VersionCommand {
    pub fn new() -> Self {
        VersionCommand
    }

    // Java: com/google/devtools/build/lib/runtime/commands/VersionCommand.java:69 VersionCommand.getInfo
    pub fn get_info(
        product_name: &str,
        info: &BlazeVersionInfo,
        gnu_format: bool,
    ) -> Option<String> {
        let summary = info.get_summary()?;
        if gnu_format {
            return Some(format!(
                "{} {}",
                product_name,
                if info.is_released_blaze() {
                    info.get_version()
                } else {
                    "no_version".to_string()
                }
            ));
        }
        Some(summary)
    }
}

impl BlazeCommand for VersionCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &crate::command_registry::VERSION_COMMAND
    }

    // Java: com/google/devtools/build/lib/runtime/commands/VersionCommand.java:62 VersionCommand.editOptions
    fn edit_options(&self, _options_parser: &mut blaze_options::OptionsParser) {}

    // Java: com/google/devtools/build/lib/runtime/commands/VersionCommand.java:65 VersionCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        // PORT: `env.getEventBus().post(new NoBuildEvent())` is skipped: Blaze has no event bus
        // yet.
        let info = Self::get_info(
            env.get_runtime().get_product_name(),
            &BlazeVersionInfo::instance(),
            options
                .get_options::<VersionCommandVersionOptions>()
                .expect("VersionCommand.VersionOptions")
                .gnu_format,
        );
        if let Some(info) = info {
            env.get_reporter().get_out_err().print_out_ln(&info);
            return BlazeCommandResult::success();
        }
        let message = "Version information not available";
        env.get_reporter().handle(&Event::error(message));
        BlazeCommandResult::failure_detail(FailureDetail {
            message: message.to_string(),
            category: Some(Category::VersionCommand(failure_details::VersionCommand {
                code: failure_details::version_command::Code::NotAvailable as i32,
            })),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GNU_FORMAT: bool = true;
    const LEGACY_FORMAT: bool = false;

    // Java: src/test/java/com/google/devtools/build/lib/runtime/commands/VersionCommandTest.java
    #[test]
    fn test_no_summary() {
        let info = BlazeVersionInfo::new(Vec::<(String, String)>::new());
        assert_eq!(
            VersionCommand::get_info("product", &info, LEGACY_FORMAT),
            None
        );
    }

    #[test]
    fn test_no_summary_gnu_format() {
        let info = BlazeVersionInfo::new(Vec::<(String, String)>::new());
        assert_eq!(VersionCommand::get_info("product", &info, GNU_FORMAT), None);
    }

    #[test]
    fn test_no_version_gnu_format() {
        let info = BlazeVersionInfo::new([(BlazeVersionInfo::BUILD_LABEL, "")]);
        assert_eq!(
            VersionCommand::get_info("product", &info, GNU_FORMAT).as_deref(),
            Some("product no_version")
        );
    }

    #[test]
    fn test_version_gnu_format() {
        let info = BlazeVersionInfo::new([(BlazeVersionInfo::BUILD_LABEL, "1.2")]);
        assert_eq!(
            VersionCommand::get_info("product", &info, GNU_FORMAT).as_deref(),
            Some("product 1.2")
        );
    }

    #[test]
    fn test_legacy_format() {
        let info =
            BlazeVersionInfo::new([(BlazeVersionInfo::BUILD_LABEL, "version"), ("More", "foo")]);
        assert_eq!(
            VersionCommand::get_info("product", &info, LEGACY_FORMAT).as_deref(),
            Some("Build label: version\nMore: foo")
        );
    }

    /// Oracle: `bazel version --gnu_format` prints `bazel 9.3.2`.
    #[test]
    fn oracle_gnu_format() {
        let info = BlazeVersionInfo::new(crate::version_info::try_get_build_info());
        assert_eq!(
            VersionCommand::get_info("bazel", &info, GNU_FORMAT).as_deref(),
            Some("bazel 9.3.2")
        );
    }
}
