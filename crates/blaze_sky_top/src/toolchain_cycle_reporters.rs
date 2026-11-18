//! Ports of `skyframe/toolchains/RegisteredExecutionPlatformsCycleReporter.java` and
//! `skyframe/toolchains/RegisteredToolchainsCycleReporter.java`: the `SingleCycleReporter`s for
//! cycles through registered execution platforms and registered toolchains.
//!
//! Java registers them in `SkyframeExecutor.createCyclesReporter`, after `TargetCycleReporter`,
//! `ActionArtifactCycleReporter` and `TestExpansionCycleReporter`:
//! `RegisteredToolchainsCycleReporter` first, then `RegisteredExecutionPlatformsCycleReporter`.

use std::sync::Arc;

use blaze_analysis::configured_target_key::{CONFIGURED_TARGET, ConfiguredTargetKey};
use blaze_analysis::toolchains::toolchain_context_key::{
    TOOLCHAIN_RESOLUTION, ToolchainContextKey,
};
use blaze_events::{Event, ExtendedEventHandler};
use blaze_sky_analysis::toolchains::registered_execution_platforms::REGISTERED_EXECUTION_PLATFORMS;
use blaze_sky_analysis::toolchains::registered_toolchains::REGISTERED_TOOLCHAINS;
use blaze_sky_analysis::toolchains::single_toolchain_resolution::SINGLE_TOOLCHAIN_RESOLUTION;
use blaze_sky_analysis::toolchains::{
    RegisteredExecutionPlatformsKey, RegisteredToolchainsKey, SingleToolchainResolutionKey,
};
use blaze_skyframe::{CycleInfo, SkyKey};

use crate::cycles_reporter::{SingleCycleReporter, print_cycle};

/// `String.format("toolchain types %s", toolchainTypes)` of a `ToolchainContextKey`: the labels
/// of its toolchain types, joined with `", "`.
fn toolchain_types_of_toolchain_context_key(toolchain_context_key: &ToolchainContextKey) -> String {
    let toolchain_types = toolchain_context_key
        .toolchain_types()
        .iter()
        .map(|requirement| requirement.toolchain_type.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!("toolchain types {}", toolchain_types)
}

/// `SingleCycleReporter` implementation that can handle cycles involving registered execution
/// platforms.
// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsCycleReporter.java:39 RegisteredExecutionPlatformsCycleReporter
pub struct RegisteredExecutionPlatformsCycleReporter;

impl RegisteredExecutionPlatformsCycleReporter {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsCycleReporter.java:42 IS_REGISTERED_EXECUTION_PLATFORMS_SKY_KEY
    fn is_registered_execution_platforms_sky_key(key: &SkyKey) -> bool {
        key.function_name() == REGISTERED_EXECUTION_PLATFORMS
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsCycleReporter.java:45 IS_CONFIGURED_TARGET_SKY_KEY
    fn is_configured_target_sky_key(key: &SkyKey) -> bool {
        key.function_name() == CONFIGURED_TARGET
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsCycleReporter.java:51 IS_TOOLCHAIN_RELATED
    fn is_toolchain_related(key: &SkyKey) -> bool {
        Self::is_registered_execution_platforms_sky_key(key)
            || key.function_name() == TOOLCHAIN_RESOLUTION
    }

    /// The `printer` function of `maybeReportCycle`.
    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsCycleReporter.java:73 RegisteredExecutionPlatformsCycleReporter.maybeReportCycle
    fn printer(input: &SkyKey) -> String {
        if let Some(ctk) = input.downcast::<ConfiguredTargetKey>() {
            let label = ctk.get_label();
            return label.to_string();
        }
        if input
            .downcast::<RegisteredExecutionPlatformsKey>()
            .is_some()
        {
            return "RegisteredExecutionPlatforms".to_owned();
        }
        if let Some(toolchain_context_key) = input.downcast::<ToolchainContextKey>() {
            return toolchain_types_of_toolchain_context_key(toolchain_context_key);
        }

        panic!("UnsupportedOperationException: {:?}", input)
    }

    /// Returns the first `SkyKey` that is an instance of `ConfiguredTargetKey` and follows
    /// `RegisteredExecutionPlatformsValue.Key`. This will loop over the cycle in case the
    /// `RegisteredExecutionPlatformsValue` is not first in the list.
    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsCycleReporter.java:114 RegisteredExecutionPlatformsCycleReporter.findRootConfiguredTarget
    fn find_root_configured_target(cycle: &[SkyKey]) -> Option<&ConfiguredTargetKey> {
        // Loop over the cycle, possibly twice, first looking for
        // RegisteredExecutionPlatformsValue, then finding the first ConfiguredTargetKey.
        let mut repv_found = false;
        for i in 0..cycle.len() * 2 {
            let sky_key = &cycle[i % cycle.len()];
            if !repv_found && Self::is_registered_execution_platforms_sky_key(sky_key) {
                repv_found = true;
            }
            if repv_found && Self::is_configured_target_sky_key(sky_key) {
                // Java: `(ConfiguredTargetKey) skyKey`.
                return Some(
                    sky_key
                        .downcast::<ConfiguredTargetKey>()
                        .unwrap_or_else(|| panic!("ClassCastException: {:?}", sky_key)),
                );
            }
        }

        None
    }
}

impl SingleCycleReporter for RegisteredExecutionPlatformsCycleReporter {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsCycleReporter.java:55 RegisteredExecutionPlatformsCycleReporter.maybeReportCycle
    fn maybe_report_cycle(
        &self,
        _top_level_key: &SkyKey,
        cycle_info: &CycleInfo,
        already_reported: bool,
        event_handler: &Arc<dyn ExtendedEventHandler>,
    ) -> bool {
        let cycle = cycle_info.get_cycle();
        if already_reported {
            return true;
        } else if !cycle.iter().any(Self::is_toolchain_related) {
            return false;
        }

        // Find the ConfiguredTargetKey, this should tell the problem.
        let Some(configured_target_key) = Self::find_root_configured_target(cycle) else {
            return false;
        };

        // TODO: jcater - Clean this up: ideally we only need to list the actual platform and the
        // direct dependency of it in the cycle.
        let mut cycle_message = String::new();
        cycle_message.push_str("Misconfigured execution platforms: ");
        cycle_message.push_str(&configured_target_key.get_label().to_string());
        cycle_message.push_str(" is declared as a platform but has inappropriate dependencies.");
        cycle_message
            .push_str(" Execution platforms should not have dependencies that themselves require");
        cycle_message.push_str(" toolchain resolution.");

        // Java: AbstractLabelCycleReporter.printCycle(cycle, cycleMessage, printer) (no
        // intermediate key is skipped).
        print_cycle(cycle, &mut cycle_message, &Self::printer, &|_| false);
        event_handler.handle(&Event::error(cycle_message));
        true
    }
}

/// `SingleCycleReporter` implementation that can handle cycles involving registered toolchains.
// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsCycleReporter.java:41 RegisteredToolchainsCycleReporter
pub struct RegisteredToolchainsCycleReporter;

impl RegisteredToolchainsCycleReporter {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsCycleReporter.java:43 IS_REGISTERED_TOOLCHAINS_SKY_KEY
    fn is_registered_toolchains_sky_key(key: &SkyKey) -> bool {
        key.function_name() == REGISTERED_TOOLCHAINS
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsCycleReporter.java:46 IS_CONFIGURED_TARGET_SKY_KEY
    fn is_configured_target_sky_key(key: &SkyKey) -> bool {
        key.function_name() == CONFIGURED_TARGET
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsCycleReporter.java:55 IS_TOOLCHAIN_RELATED
    fn is_toolchain_related(key: &SkyKey) -> bool {
        Self::is_registered_toolchains_sky_key(key)
            || key.function_name() == SINGLE_TOOLCHAIN_RESOLUTION
            || key.function_name() == TOOLCHAIN_RESOLUTION
    }

    /// The `printer` function of `maybeReportCycle`.
    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsCycleReporter.java:79 RegisteredToolchainsCycleReporter.maybeReportCycle
    fn printer(input: &SkyKey) -> String {
        if let Some(ctk) = input.downcast::<ConfiguredTargetKey>() {
            let label = ctk.get_label();
            return label.to_string();
        }
        if input.downcast::<RegisteredToolchainsKey>().is_some() {
            return "RegisteredToolchains".to_owned();
        }
        if let Some(single_toolchain_resolution_key) =
            input.downcast::<SingleToolchainResolutionKey>()
        {
            let toolchain_type = &single_toolchain_resolution_key
                .toolchain_type()
                .toolchain_type;
            return format!("toolchain type {}", toolchain_type);
        }
        if let Some(toolchain_context_key) = input.downcast::<ToolchainContextKey>() {
            return toolchain_types_of_toolchain_context_key(toolchain_context_key);
        }

        panic!("UnsupportedOperationException: {:?}", input)
    }

    /// Returns the first `SkyKey` that is an instance of `ConfiguredTargetKey` and follows
    /// `RegisteredToolchainsValue.Key`. This will loop over the cycle in case the
    /// `RegisteredToolchainsValue` is not first in the list.
    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsCycleReporter.java:121 RegisteredToolchainsCycleReporter.findRootConfiguredTarget
    fn find_root_configured_target(cycle: &[SkyKey]) -> Option<&ConfiguredTargetKey> {
        // Loop over the cycle, possibly twice, first looking for RegisteredToolchainsValue, then
        // finding the first ConfiguredTargetKey.
        let mut rtv_found = false;
        for i in 0..cycle.len() * 2 {
            let sky_key = &cycle[i % cycle.len()];
            if !rtv_found && Self::is_registered_toolchains_sky_key(sky_key) {
                rtv_found = true;
            }
            if rtv_found && Self::is_configured_target_sky_key(sky_key) {
                // Java: `(ConfiguredTargetKey) skyKey`.
                return Some(
                    sky_key
                        .downcast::<ConfiguredTargetKey>()
                        .unwrap_or_else(|| panic!("ClassCastException: {:?}", sky_key)),
                );
            }
        }

        None
    }
}

impl SingleCycleReporter for RegisteredToolchainsCycleReporter {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsCycleReporter.java:61 RegisteredToolchainsCycleReporter.maybeReportCycle
    fn maybe_report_cycle(
        &self,
        _top_level_key: &SkyKey,
        cycle_info: &CycleInfo,
        already_reported: bool,
        event_handler: &Arc<dyn ExtendedEventHandler>,
    ) -> bool {
        let cycle = cycle_info.get_cycle();
        if already_reported {
            return true;
        } else if !cycle.iter().any(Self::is_toolchain_related) {
            return false;
        }

        // Find the ConfiguredTargetKey, this should tell the problem.
        let Some(configured_target_key) = Self::find_root_configured_target(cycle) else {
            return false;
        };

        let mut cycle_message = String::new();
        cycle_message.push_str("Misconfigured toolchains: ");
        cycle_message.push_str(&configured_target_key.get_label().to_string());
        cycle_message.push_str(" is declared as a toolchain but has inappropriate dependencies.");
        cycle_message.push_str(" Declared toolchains should be created with the 'toolchain' rule");
        cycle_message
            .push_str(" and should not have dependencies that themselves require toolchains.");

        // Java: AbstractLabelCycleReporter.printCycle(cycle, cycleMessage, printer) (no
        // intermediate key is skipped).
        print_cycle(cycle, &mut cycle_message, &Self::printer, &|_| false);
        event_handler.handle(&Event::error(cycle_message));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blaze_cmdline::Label;
    use blaze_events::StoredEventHandler;
    use blaze_sky_analysis::toolchains::platform_keys::empty_options_configuration_key;

    fn configured_target_key(label: &str) -> SkyKey {
        SkyKey::new(
            ConfiguredTargetKey::builder()
                .set_label(Label::parse_canonical(label).expect("label"))
                .set_configuration_key(Some(empty_options_configuration_key()))
                .build(),
        )
    }

    fn toolchain_context_key() -> SkyKey {
        let mut builder = ToolchainContextKey::key();
        builder.configuration_key(empty_options_configuration_key());
        SkyKey::new(builder.build())
    }

    fn registered_execution_platforms_key() -> SkyKey {
        SkyKey::new(RegisteredExecutionPlatformsKey::key(
            empty_options_configuration_key(),
            false,
        ))
    }

    fn report(reporter: &dyn SingleCycleReporter, cycle: Vec<SkyKey>) -> (bool, Vec<String>) {
        let stored = Arc::new(StoredEventHandler::new());
        let event_handler: Arc<dyn ExtendedEventHandler> = stored.clone();
        let top_level_key = cycle[0].clone();
        let reported = reporter.maybe_report_cycle(
            &top_level_key,
            &CycleInfo::create_cycle_info_without_path(cycle),
            false,
            &event_handler,
        );
        let messages = stored
            .get_events()
            .iter()
            .map(|e| e.get_message().into_owned())
            .collect();
        (reported, messages)
    }

    /// The message of the oracle (Bazel 9.3.2) for `build //:x --extra_execution_platforms=//:x`
    /// with `filegroup(name = "x")` (scripts/w4/scenarios/top-bad-platform, step 8): the cycle is
    /// the platform's configured target, its toolchain context (no toolchain types) and the
    /// registered execution platforms.
    #[test]
    fn execution_platform_cycle_message_of_the_oracle() {
        let (reported, messages) = report(
            &RegisteredExecutionPlatformsCycleReporter,
            vec![
                configured_target_key("//:x"),
                toolchain_context_key(),
                registered_execution_platforms_key(),
            ],
        );
        assert!(reported);
        assert_eq!(
            messages,
            vec![
                "Misconfigured execution platforms: //:x is declared as a platform but has \
                 inappropriate dependencies. Execution platforms should not have dependencies \
                 that themselves require toolchain resolution.\n\
                 .-> //:x\n\
                 |   toolchain types \n\
                 |   RegisteredExecutionPlatforms\n\
                 `-- //:x"
                    .to_owned()
            ]
        );
    }

    /// The same cycle starting at the registered execution platforms (the oracle's message for
    /// `build //:needs_tt --extra_toolchains=//:tc --extra_execution_platforms=//:x`, step 16).
    #[test]
    fn execution_platform_cycle_starting_at_the_registered_execution_platforms() {
        let (reported, messages) = report(
            &RegisteredExecutionPlatformsCycleReporter,
            vec![
                registered_execution_platforms_key(),
                configured_target_key("//:x"),
                toolchain_context_key(),
            ],
        );
        assert!(reported);
        assert_eq!(
            messages,
            vec![
                "Misconfigured execution platforms: //:x is declared as a platform but has \
                 inappropriate dependencies. Execution platforms should not have dependencies \
                 that themselves require toolchain resolution.\n\
                 .-> RegisteredExecutionPlatforms\n\
                 |   //:x\n\
                 |   toolchain types \n\
                 `-- RegisteredExecutionPlatforms"
                    .to_owned()
            ]
        );
    }

    /// `maybeReportCycle` declines a cycle without a toolchain-related key, and one in which no
    /// configured target follows the registered execution platforms (Java lines 64 and 70).
    #[test]
    fn execution_platform_reporter_declines_other_cycles() {
        let (reported, messages) = report(
            &RegisteredExecutionPlatformsCycleReporter,
            vec![configured_target_key("//:a"), configured_target_key("//:b")],
        );
        assert!(!reported);
        assert!(messages.is_empty());

        let (reported, messages) = report(
            &RegisteredExecutionPlatformsCycleReporter,
            vec![configured_target_key("//:a"), toolchain_context_key()],
        );
        assert!(!reported);
        assert!(messages.is_empty());

        // The registered toolchains reporter declines the execution platform cycle: it has a
        // TOOLCHAIN_RESOLUTION key but no REGISTERED_TOOLCHAINS key.
        let (reported, _) = report(
            &RegisteredToolchainsCycleReporter,
            vec![
                configured_target_key("//:x"),
                toolchain_context_key(),
                registered_execution_platforms_key(),
            ],
        );
        assert!(!reported);
    }

    /// `alreadyReported`: the reporter claims the cycle without printing it again (Java line 62).
    #[test]
    fn already_reported_cycle_is_claimed_silently() {
        let stored = Arc::new(StoredEventHandler::new());
        let event_handler: Arc<dyn ExtendedEventHandler> = stored.clone();
        let cycle = vec![configured_target_key("//:a")];
        assert!(
            RegisteredExecutionPlatformsCycleReporter.maybe_report_cycle(
                &cycle[0],
                &CycleInfo::create_cycle_info_without_path(cycle.clone()),
                true,
                &event_handler,
            )
        );
        assert!(RegisteredToolchainsCycleReporter.maybe_report_cycle(
            &cycle[0],
            &CycleInfo::create_cycle_info_without_path(cycle.clone()),
            true,
            &event_handler,
        ));
        assert!(stored.get_events().is_empty());
    }

    /// `RegisteredToolchainsCycleReporter`: the Java message (lines 100-106) and printer.
    #[test]
    fn registered_toolchains_cycle_message() {
        let (reported, messages) = report(
            &RegisteredToolchainsCycleReporter,
            vec![
                SkyKey::new(RegisteredToolchainsKey::key(
                    empty_options_configuration_key(),
                    false,
                )),
                configured_target_key("//:tc"),
                toolchain_context_key(),
            ],
        );
        assert!(reported);
        assert_eq!(
            messages,
            vec![
                "Misconfigured toolchains: //:tc is declared as a toolchain but has \
                 inappropriate dependencies. Declared toolchains should be created with the \
                 'toolchain' rule and should not have dependencies that themselves require \
                 toolchains.\n\
                 .-> RegisteredToolchains\n\
                 |   //:tc\n\
                 |   toolchain types \n\
                 `-- RegisteredToolchains"
                    .to_owned()
            ]
        );
    }
}
