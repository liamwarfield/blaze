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
// Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:124 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.IndentLevel
#[derive(Clone, Copy)]
enum IndentLevel {
    TargetPlatformLevel,
    ToolchainLevel,
    ExecutionPlatformLevel,
}

impl IndentLevel {
    fn indent(self) -> &'static str {
        match self {
            IndentLevel::TargetPlatformLevel => "",
            IndentLevel::ToolchainLevel => "  ",
            IndentLevel::ExecutionPlatformLevel => "    ",
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:33 SingleToolchainResolutionDebugPrinter
/// A helper for printing debug messages from single toolchain resolution.
pub struct SingleToolchainResolutionDebugPrinter {
    /// `EventHandlerImpl` when true, `NoopPrinter` when false.
    enabled: bool,
    resolution_trace: Vec<String>,
}

impl SingleToolchainResolutionDebugPrinter {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:34 SingleToolchainResolutionDebugPrinter.create
    pub fn create(debug: bool) -> Self {
        SingleToolchainResolutionDebugPrinter {
            enabled: debug,
            resolution_trace: Vec::new(),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:149 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.debugMessage
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

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:156 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.finishDebugging
    pub fn finish_debugging(&self, event_handler: &dyn EventHandler) {
        if !self.enabled {
            return;
        }
        event_handler.handle(&Event::info(self.resolution_trace.join("\n")));
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:161 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.describeRejectedToolchains
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

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:173 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.startToolchainResolution
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

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:182 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.reportCompatibleTargetPlatform
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

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:192 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.reportSkippedExecutionPlatformSeen
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

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:200 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.reportSkippedExecutionPlatformDisallowed
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

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:211 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.reportCompatibleExecutionPlatform
    pub fn report_compatible_execution_platform(&mut self, execution_platform: &Label) {
        if !self.enabled {
            return;
        }
        self.debug_message(
            IndentLevel::ExecutionPlatformLevel,
            format!("Compatible execution platform {}", execution_platform),
        );
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:219 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.reportResolvedToolchains
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

    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionDebugPrinter.java:282 SingleToolchainResolutionDebugPrinter.EventHandlerImpl.reportDone
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

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:30 ToolchainResolutionDebugPrinter
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
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:32 ToolchainResolutionDebugPrinter.create
    pub fn create(debug: bool) -> Self {
        ToolchainResolutionDebugPrinter { enabled: debug }
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:84 ToolchainResolutionDebugPrinter.EventHandlerImpl.debugEnabled
    pub fn debug_enabled(&self) -> bool {
        self.enabled
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:94 ToolchainResolutionDebugPrinter.EventHandlerImpl.debugMessage
    fn debug_message(&self, event_handler: &dyn EventHandler, message: String) {
        if self.enabled {
            event_handler.handle(&Event::info(message));
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:99 ToolchainResolutionDebugPrinter.EventHandlerImpl.reportSelectedToolchains
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

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionDebugPrinter.java:116 ToolchainResolutionDebugPrinter.EventHandlerImpl.reportRemovedExecutionPlatform
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
