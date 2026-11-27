//! Ports of `skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java` and
//! `skyframe/toolchains/ToolchainResolutionDebugPrinter.java`: the `--toolchain_resolution_debug`
//! messages.
//!
//! Java's sealed interfaces have a do-nothing implementation (`NoopPrinter`) and an
//! `EventHandlerImpl`; here one struct with an `enabled` flag plays both (every method returns at
//! once when it is off).

use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::platform::{
    ConstraintCollection, ConstraintSettingInfo, ConstraintValueInfo, PlatformInfo,
    ToolchainTypeInfo,
};
use blaze_cmdline::Label;
use blaze_events::{Event, EventHandler};
use indexmap::{IndexMap, IndexSet};

/// The three indentation levels used in `debugMessage`.
// Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:121 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.IndentLevel
#[derive(Clone, Copy)]
enum IndentLevel {
    TargetPlatformLevel,
    ToolchainLevel,
    ExecutionPlatformLevel,
}

impl IndentLevel {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:132 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.IndentLevel.indent
    fn indent(self) -> &'static str {
        match self {
            IndentLevel::TargetPlatformLevel => "",
            IndentLevel::ToolchainLevel => "  ",
            IndentLevel::ExecutionPlatformLevel => "    ",
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:34 SingleToolchainResolutionDebugPrinter
/// A helper for printing debug messages from single toolchain resolution.
pub struct SingleToolchainResolutionDebugPrinter {
    /// `EventHandlerImpl` when true, `NoopPrinter` when false.
    enabled: bool,
    resolution_trace: Vec<String>,
}

impl SingleToolchainResolutionDebugPrinter {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:35 SingleToolchainResolutionDebugPrinter.create
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:75 SingleToolchainResolutionDebugPrinter.NoopPrinter.<init>
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:140 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.<init>
    pub fn create(debug: bool) -> Self {
        SingleToolchainResolutionDebugPrinter {
            enabled: debug,
            resolution_trace: Vec::new(),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:145 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.debugMessage
    fn debug_message(&mut self, indent: IndentLevel, message: String) {
        let padding = if self.resolution_trace.is_empty() {
            String::new()
        } else {
            " ".repeat("INFO: ".len())
        };
        self.resolution_trace.push(format!(
            "{}ToolchainResolution: {}{}",
            padding,
            indent.indent(),
            message
        ));
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:152 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.finishDebugging
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:43 SingleToolchainResolutionDebugPrinter.finishDebugging
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:78 SingleToolchainResolutionDebugPrinter.NoopPrinter.finishDebugging
    pub fn finish_debugging(&self, event_handler: &dyn EventHandler) {
        if !self.enabled {
            return;
        }
        event_handler.handle(&Event::info(self.resolution_trace.join("\n")));
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:157 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.describeRejectedToolchains
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:45 SingleToolchainResolutionDebugPrinter.describeRejectedToolchains
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:81 SingleToolchainResolutionDebugPrinter.NoopPrinter.describeRejectedToolchains
    pub fn describe_rejected_toolchains(&mut self, rejected_toolchains: &IndexMap<Label, String>) {
        if !self.enabled {
            return;
        }
        for (toolchain_label, message) in rejected_toolchains {
            self.debug_message(
                IndentLevel::ToolchainLevel,
                format!("Rejected toolchain {}; {}", toolchain_label, message),
            );
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:169 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.startToolchainResolution
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:47 SingleToolchainResolutionDebugPrinter.startToolchainResolution
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:84 SingleToolchainResolutionDebugPrinter.NoopPrinter.startToolchainResolution
    pub fn start_toolchain_resolution(&mut self, toolchain_type: &Label, target_platform: &Label) {
        if !self.enabled {
            return;
        }
        self.debug_message(
            IndentLevel::TargetPlatformLevel,
            format!(
                "Performing resolution of {} for target platform {}",
                toolchain_type, target_platform
            ),
        );
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:178 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.reportCompatibleTargetPlatform
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:49 SingleToolchainResolutionDebugPrinter.reportCompatibleTargetPlatform
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:87 SingleToolchainResolutionDebugPrinter.NoopPrinter.reportCompatibleTargetPlatform
    pub fn report_compatible_target_platform(
        &mut self,
        target_label: &Label,
        resolved_toolchain_label: &Label,
    ) {
        if !self.enabled {
            return;
        }
        self.debug_message(
            IndentLevel::ToolchainLevel,
            format!(
                "Toolchain {} (resolves to {}) is compatible with target platform, searching for execution platforms:",
                target_label, resolved_toolchain_label
            ),
        );
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:188 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.reportSkippedExecutionPlatformSeen
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:51 SingleToolchainResolutionDebugPrinter.reportSkippedExecutionPlatformSeen
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:90 SingleToolchainResolutionDebugPrinter.NoopPrinter.reportSkippedExecutionPlatformSeen
    pub fn report_skipped_execution_platform_seen(&mut self, execution_platform: &Label) {
        if !self.enabled {
            return;
        }
        self.debug_message(
            IndentLevel::ExecutionPlatformLevel,
            format!(
                "Skipping execution platform {}; it has already selected a toolchain",
                execution_platform
            ),
        );
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:196 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.reportSkippedExecutionPlatformDisallowed
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:53 SingleToolchainResolutionDebugPrinter.reportSkippedExecutionPlatformDisallowed
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:93 SingleToolchainResolutionDebugPrinter.NoopPrinter.reportSkippedExecutionPlatformDisallowed
    pub fn report_skipped_execution_platform_disallowed(
        &mut self,
        execution_platform: &Label,
        toolchain_type: &Label,
    ) {
        if !self.enabled {
            return;
        }
        self.debug_message(
            IndentLevel::ExecutionPlatformLevel,
            format!(
                "Skipping execution platform {}; its allowed toolchain types does not contain the current toolchain type {}",
                execution_platform, toolchain_type
            ),
        );
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:207 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.reportCompatibleExecutionPlatform
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:55 SingleToolchainResolutionDebugPrinter.reportCompatibleExecutionPlatform
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:97 SingleToolchainResolutionDebugPrinter.NoopPrinter.reportCompatibleExecutionPlatform
    pub fn report_compatible_execution_platform(&mut self, execution_platform: &Label) {
        if !self.enabled {
            return;
        }
        self.debug_message(
            IndentLevel::ExecutionPlatformLevel,
            format!("Compatible execution platform {}", execution_platform),
        );
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:215 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.reportResolvedToolchains
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:57 SingleToolchainResolutionDebugPrinter.reportResolvedToolchains
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:100 SingleToolchainResolutionDebugPrinter.NoopPrinter.reportResolvedToolchains
    pub fn report_resolved_toolchains(
        &mut self,
        resolved_toolchains: &IndexMap<ConfiguredTargetKey, Label>,
        target_platform: &Label,
        toolchain_type: &Label,
    ) {
        if !self.enabled {
            return;
        }
        if resolved_toolchains.is_empty() {
            self.debug_message(
                IndentLevel::TargetPlatformLevel,
                format!(
                    "No {} toolchain found for target platform {}.",
                    toolchain_type, target_platform
                ),
            );
        } else {
            self.debug_message(
                IndentLevel::TargetPlatformLevel,
                format!(
                    "Recap of selected {} toolchains for target platform {}:",
                    toolchain_type, target_platform
                ),
            );
            for (execution_platform_key, resolved_toolchain_label) in resolved_toolchains {
                self.debug_message(
                    IndentLevel::ToolchainLevel,
                    format!(
                        "Selected {} to run on execution platform {}",
                        resolved_toolchain_label,
                        execution_platform_key.get_label()
                    ),
                );
            }
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:242 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.reportMismatchedSettings
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:62 SingleToolchainResolutionDebugPrinter.reportMismatchedSettings
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:106 SingleToolchainResolutionDebugPrinter.NoopPrinter.reportMismatchedSettings
    pub fn report_mismatched_settings(
        &mut self,
        toolchain_constraints: &ConstraintCollection,
        is_target_platform: bool,
        platform: &PlatformInfo,
        target_label: &Label,
        resolved_toolchain_label: &Label,
        mismatch_settings_with_default: &IndexSet<ConstraintSettingInfo>,
    ) {
        if !self.enabled {
            return;
        }
        if !mismatch_settings_with_default.is_empty() {
            let mut mismatch_values = mismatch_settings_with_default
                .iter()
                .filter(|s| toolchain_constraints.has(s))
                .map(|s| {
                    toolchain_constraints
                        .get(s)
                        .expect("NullPointerException: toolchainConstraints.get(s)")
                        .label()
                        .get_name()
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join(", ");
            if !mismatch_values.is_empty() {
                mismatch_values = format!("; mismatching values: {}", mismatch_values);
            }

            let mut missing_settings = mismatch_settings_with_default
                .iter()
                .filter(|s| !toolchain_constraints.has(s))
                .map(|s| s.label().get_name().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            if !missing_settings.is_empty() {
                missing_settings = format!("; missing: {}", missing_settings);
            }
            if is_target_platform {
                self.debug_message(
                    IndentLevel::ToolchainLevel,
                    format!(
                        "Rejected toolchain {} (resolves to {}) {}",
                        target_label,
                        resolved_toolchain_label,
                        format!("{}{}", mismatch_values, missing_settings)
                    ),
                );
            } else {
                self.debug_message(
                    IndentLevel::ExecutionPlatformLevel,
                    format!(
                        "Incompatible execution platform {}{}",
                        platform.label(),
                        format!("{}{}", mismatch_values, missing_settings)
                    ),
                );
            }
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:285 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.reportDone
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:70 SingleToolchainResolutionDebugPrinter.reportDone
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:115 SingleToolchainResolutionDebugPrinter.NoopPrinter.reportDone
    pub fn report_done(&mut self, toolchain_type: &Label) {
        if !self.enabled {
            return;
        }
        self.debug_message(
            IndentLevel::ToolchainLevel,
            format!(
                "All execution platforms have been assigned a {} toolchain, stopping",
                toolchain_type
            ),
        );
    }
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:31 ToolchainResolutionDebugPrinter
/// A helper for printing debug messages from toolchain resolution.
///
/// PORT: Java's `EventHandlerImpl` keeps `env.getListener()`; the callers hold the Skyframe
/// environment mutably while they report, so each report takes the event handler instead.
#[derive(Clone, Copy)]
pub struct ToolchainResolutionDebugPrinter {
    /// `EventHandlerImpl` when true, `NoopPrinter` when false.
    enabled: bool,
}

impl ToolchainResolutionDebugPrinter {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:33 ToolchainResolutionDebugPrinter.create
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:57 ToolchainResolutionDebugPrinter.NoopPrinter.<init>
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:88 ToolchainResolutionDebugPrinter.EventHandlerImpl.<init>
    pub fn create(debug: bool) -> Self {
        ToolchainResolutionDebugPrinter { enabled: debug }
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:82 ToolchainResolutionDebugPrinter.EventHandlerImpl.debugEnabled
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:40 ToolchainResolutionDebugPrinter.debugEnabled
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:60 ToolchainResolutionDebugPrinter.NoopPrinter.debugEnabled
    pub fn debug_enabled(&self) -> bool {
        self.enabled
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:93 ToolchainResolutionDebugPrinter.EventHandlerImpl.debugMessage
    fn debug_message(&self, event_handler: &dyn EventHandler, message: String) {
        if self.enabled {
            event_handler.handle(&Event::info(message));
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:98 ToolchainResolutionDebugPrinter.EventHandlerImpl.reportSelectedToolchains
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:43 ToolchainResolutionDebugPrinter.reportSelectedToolchains
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:65 ToolchainResolutionDebugPrinter.NoopPrinter.reportSelectedToolchains
    /// Report on which toolchains were selected. `toolchain_type_to_resolved` is the entries of
    /// the `ImmutableSetMultimap`, in its order.
    pub fn report_selected_toolchains(
        &self,
        event_handler: &dyn EventHandler,
        target_platform: &Label,
        execution_platform: &Label,
        toolchain_type_to_resolved: &[(ToolchainTypeInfo, Label)],
    ) {
        if !self.enabled {
            return;
        }
        let selected_toolchains = toolchain_type_to_resolved
            .iter()
            .map(|(k, v)| format!("type {} -> toolchain {}", k.type_label(), v))
            .collect::<Vec<_>>()
            .join(", ");
        self.debug_message(
            event_handler,
            format!(
                "ToolchainResolution: Target platform {}: Selected execution platform {}, {}",
                target_platform, execution_platform, selected_toolchains
            ),
        );
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:115 ToolchainResolutionDebugPrinter.EventHandlerImpl.reportRemovedExecutionPlatform
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:49 ToolchainResolutionDebugPrinter.reportRemovedExecutionPlatform
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:71 ToolchainResolutionDebugPrinter.NoopPrinter.reportRemovedExecutionPlatform
    /// Report on an execution platform that was skipped due to constraint mismatches.
    pub fn report_removed_execution_platform(
        &self,
        event_handler: &dyn EventHandler,
        label: &Label,
        missing_constraints: &[ConstraintValueInfo],
    ) {
        // TODO: jcater - Make this one line listing all constraints.
        for constraint in missing_constraints {
            // The value for this setting is not present in the platform, or doesn't match the
            // expected value.
            self.debug_message(
                event_handler,
                format!(
                    "ToolchainResolution: Removed execution platform {} from available execution platforms, it is missing constraint {}",
                    label,
                    constraint.label()
                ),
            );
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:129 ToolchainResolutionDebugPrinter.EventHandlerImpl.reportRejectedExecutionPlatforms
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:52 ToolchainResolutionDebugPrinter.reportRejectedExecutionPlatforms
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:75 ToolchainResolutionDebugPrinter.NoopPrinter.reportRejectedExecutionPlatforms
    pub fn report_rejected_execution_platforms(
        &self,
        event_handler: &dyn EventHandler,
        rejected_execution_platforms: &IndexMap<Label, String>,
    ) {
        for (toolchain_label, message) in rejected_execution_platforms {
            self.debug_message(
                event_handler,
                format!(
                    "ToolchainResolution: Rejected execution platform {}; {}",
                    toolchain_label, message
                ),
            );
        }
    }
}
