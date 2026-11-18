//! Ports of `analysis/constraints/TopLevelConstraintSemantics.java` and
//! `analysis/constraints/PlatformRestrictionsResult.java`: constraint semantics that apply to
//! top-level targets.
//!
//! Top-level targets are "special" because they have no parents that can assert expected
//! environment compatibility. So these expectations have to be declared by other means.
//!
//! For all other targets see `ConstraintSemantics`.

use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::Label;
use blaze_events::{Event, ExtendedEventHandler};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use indexmap::IndexSet;

use crate::config::incompatible_platform_provider::{
    INCOMPATIBLE_PLATFORM_PROVIDER, IncompatiblePlatformProvider,
};
use crate::configured_target::ConfiguredTarget;
use crate::constraints::rule_context_constraint_semantics::RuleContextConstraintSemantics;
use crate::info::downcast_info;
use crate::platform::constraint_value_info::ConstraintValueInfo;

/// Targets that have additional restrictions based on the current platform.
// Java: com/google/devtools/build/lib/analysis/constraints/PlatformRestrictionsResult.java:29 PlatformRestrictionsResult
#[derive(Clone, Debug, Default)]
pub struct PlatformRestrictionsResult {
    /// Targets that need be skipped.
    targets_to_skip: Vec<ConfiguredTarget>,
    /// Targets that should be skipped, but were explicitly requested on the command line.
    targets_with_errors: Vec<ConfiguredTarget>,
}

impl PlatformRestrictionsResult {
    // Java: com/google/devtools/build/lib/analysis/constraints/PlatformRestrictionsResult.java:29 PlatformRestrictionsResult.targetsToSkip
    pub fn targets_to_skip(&self) -> &[ConfiguredTarget] {
        &self.targets_to_skip
    }

    // Java: com/google/devtools/build/lib/analysis/constraints/PlatformRestrictionsResult.java:29 PlatformRestrictionsResult.targetsWithErrors
    pub fn targets_with_errors(&self) -> &[ConfiguredTarget] {
        &self.targets_with_errors
    }
}

/// Tells the compatibility of a ConfiguredTarget with the platform.
// Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:563 TopLevelConstraintSemantics.PlatformCompatibility
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum PlatformCompatibility {
    COMPATIBLE,
    INCOMPATIBLE_IMPLICIT,
    INCOMPATIBLE_EXPLICIT,
}

/// Tells the compatibility of a ConfiguredTarget with the target environment.
// Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:543 TopLevelConstraintSemantics.EnvironmentCompatibility
#[derive(Clone, Debug)]
pub struct EnvironmentCompatibility {
    is_compatible: bool,
}

impl EnvironmentCompatibility {
    // Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:546 TopLevelConstraintSemantics.EnvironmentCompatibility.compatible
    pub fn compatible() -> EnvironmentCompatibility {
        EnvironmentCompatibility {
            is_compatible: true,
        }
    }

    pub fn is_compatible(&self) -> bool {
        self.is_compatible
    }
}

/// For Exceptions that arise during the compatibility checking of a target.
// Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:570 TopLevelConstraintSemantics.TargetCompatibilityCheckException
#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct TargetCompatibilityCheckException {
    message: String,
    failure_detail: FailureDetail,
}

impl TargetCompatibilityCheckException {
    // Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:573 TopLevelConstraintSemantics.TargetCompatibilityCheckException.<init>
    pub fn new(message: String, failure_detail: FailureDetail) -> Self {
        TargetCompatibilityCheckException {
            message,
            failure_detail,
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:584 TopLevelConstraintSemantics.TargetCompatibilityCheckException.getFailureDetail
    pub fn get_failure_detail(&self) -> &FailureDetail {
        &self.failure_detail
    }
}

/// `ViewCreationFailedException` as `TopLevelConstraintSemantics` throws it: the message and the
/// failure detail.
// Java: com/google/devtools/build/lib/analysis/ViewCreationFailedException.java:26 ViewCreationFailedException
#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct ViewCreationFailedException {
    pub message: String,
    pub failure_detail: FailureDetail,
}

/// Constraint semantics that apply to top-level targets.
///
/// PORT: Java's instance holds the `RuleContextConstraintSemantics`, the `PackageManager` and the
/// evaluator for the target environment checks (`checkTargetEnvironmentRestrictions`); those are
/// not ported (see [`TopLevelConstraintSemantics::compatibility_with_target_environment`]), so
/// the instance only holds the event handler.
// Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:65 TopLevelConstraintSemantics
pub struct TopLevelConstraintSemantics<'a> {
    event_handler: &'a dyn ExtendedEventHandler,
}

impl<'a> TopLevelConstraintSemantics<'a> {
    /// Constructor with helper classes for loading targets.
    // Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:80 TopLevelConstraintSemantics.<init>
    pub fn new(event_handler: &'a dyn ExtendedEventHandler) -> Self {
        TopLevelConstraintSemantics { event_handler }
    }

    /// Returns the compatibility of a ConfiguredTarget with the platform.
    ///
    /// See [`TopLevelConstraintSemantics::check_platform_restrictions`].
    // Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:109 TopLevelConstraintSemantics.compatibilityWithPlatformRestrictions
    pub fn compatibility_with_platform_restrictions(
        configured_target: &ConfiguredTarget,
        event_handler: &dyn ExtendedEventHandler,
        eagerly_throw_error: bool,
        explicitly_requested: bool,
        skip_incompatible_explicit_targets: bool,
    ) -> Result<PlatformCompatibility, TargetCompatibilityCheckException> {
        let incompatible_check_result =
            RuleContextConstraintSemantics::check_for_incompatibility(configured_target);
        if !incompatible_check_result.is_incompatible() {
            return Ok(PlatformCompatibility::COMPATIBLE);
        }

        // We need the label in unambiguous form here. I.e. with the "@" prefix for targets in the
        // main repository. explicitTargetPatterns is also already in the unambiguous form to make
        // comparison succeed regardless of the provided form.
        if !skip_incompatible_explicit_targets && explicitly_requested {
            if eagerly_throw_error {
                // Use the slightly simpler form for printing error messages. I.e. no "@" prefix
                // for targets in the main repository.
                return Err(get_exception_for_explicitly_requested_incompatible_target(
                    configured_target,
                    incompatible_check_result.underlying_target(),
                ));
            }
            event_handler.handle(&Event::warn(get_incompatible_message(
                configured_target,
                incompatible_check_result.underlying_target(),
            )));
            return Ok(PlatformCompatibility::INCOMPATIBLE_EXPLICIT);
        }
        // We can safely skip this target if it wasn't explicitly requested or we've been
        // instructed to skip explicitly requested targets.
        Ok(PlatformCompatibility::INCOMPATIBLE_IMPLICIT)
    }

    /// Returns the compatibility with the target environment.
    ///
    /// See `checkTargetEnvironmentRestrictions`.
    ///
    /// PORT: the checks of an environment-restricted build (`--target_environment`:
    /// `getMissingEnvironments`, `getErrorMessageForTarget`, and with them
    /// `checkTargetEnvironmentRestrictions`' `TARGETS_MISSING_ENVIRONMENTS` failure) are not
    /// ported: a configuration that enforces constraints and names target environments fails
    /// loudly here.
    // Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:172 TopLevelConstraintSemantics.compatibilityWithTargetEnvironment
    pub fn compatibility_with_target_environment(
        configured_target: &ConfiguredTarget,
        build_configuration_value: Option<&BuildConfigurationValue>,
    ) -> Result<EnvironmentCompatibility, TargetCompatibilityCheckException> {
        // TODO(bazel-team): support file targets (they should apply package-default constraints).
        let Some(build_configuration_value) = build_configuration_value else {
            return Ok(EnvironmentCompatibility::compatible());
        };
        let core_options = crate::rule::config_util::core_options(build_configuration_value);
        if !core_options.enforce_constraints || core_options.target_environments.is_empty() {
            return Ok(EnvironmentCompatibility::compatible());
        }
        blaze_util::unported!(
            "TopLevelConstraintSemantics.getMissingEnvironments",
            "--target_environment checks of top-level target {}",
            configured_target.get_label()
        )
    }

    /// Checks that the all top-level targets are compatible with the target platform.
    ///
    /// If any target doesn't support the target platform it will be either marked as "to be
    /// skipped" or marked as "errored".
    ///
    /// Targets that are incompatible with the target platform and are not explicitly requested
    /// on the command line should be skipped.
    ///
    /// Targets that are incompatible with the target platform and *are* explicitly requested on
    /// the command line are errored unless --skip_incompatible_explicit_targets is enabled.
    /// Having one or more errored targets will cause the entire build to fail with an error
    /// message.
    ///
    /// `explicit_target_patterns`: the set of explicit target patterns specified by the user on
    /// the command line. Returns the set of to-be-skipped and errored top-level targets, or the
    /// `ViewCreationFailedException` if any top-level target was explicitly requested on the
    /// command line.
    // Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:241 TopLevelConstraintSemantics.checkPlatformRestrictions
    pub fn check_platform_restrictions(
        &self,
        top_level_targets: &[ConfiguredTarget],
        explicit_target_patterns: &IndexSet<Label>,
        keep_going: bool,
        skip_incompatible_explicit_targets: bool,
    ) -> Result<PlatformRestrictionsResult, ViewCreationFailedException> {
        let mut incompatible_targets: Vec<ConfiguredTarget> = Vec::new();
        let mut incompatible_but_requested_targets: Vec<ConfiguredTarget> = Vec::new();

        for target in top_level_targets {
            let platform_compatibility = match Self::compatibility_with_platform_restrictions(
                target,
                self.event_handler,
                /* eagerly_throw_error= */ !keep_going,
                explicit_target_patterns.contains(&target.get_original_label()),
                skip_incompatible_explicit_targets,
            ) {
                Ok(c) => c,
                // Java: `new ViewCreationFailedException(e.getFailureDetail(), /*cause=*/ e)`:
                // the message is the cause's.
                Err(e) => {
                    return Err(ViewCreationFailedException {
                        message: e.get_message().to_owned(),
                        failure_detail: e.get_failure_detail().clone(),
                    });
                }
            };
            if platform_compatibility == PlatformCompatibility::INCOMPATIBLE_EXPLICIT {
                incompatible_but_requested_targets.push(target.clone());
            } else if platform_compatibility == PlatformCompatibility::INCOMPATIBLE_IMPLICIT {
                incompatible_targets.push(target.clone());
            }
        }

        Ok(PlatformRestrictionsResult {
            targets_to_skip: incompatible_targets,
            targets_with_errors: incompatible_but_requested_targets,
        })
    }

    /// Checks that if this is an environment-restricted build, all top-level targets support
    /// expected top-level environments. Expected top-level environments can be declared
    /// explicitly through `--target_environment` or implicitly through
    /// `--auto_cpu_environment_group`. For the latter, top-level targets must be compatible with
    /// the build's target configuration CPU.
    ///
    /// `configuration_of` is Java's `getConfigurationValue` (the evaluator's existing value of
    /// the target's configuration key).
    ///
    /// Returns the set of bad top-level targets.
    // Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:370 TopLevelConstraintSemantics.checkTargetEnvironmentRestrictions
    pub fn check_target_environment_restrictions(
        &self,
        top_level_targets: &[ConfiguredTarget],
        configuration_of: &dyn Fn(
            &ConfiguredTarget,
        ) -> Option<std::sync::Arc<BuildConfigurationValue>>,
    ) -> Result<Vec<ConfiguredTarget>, ViewCreationFailedException> {
        let mut bad_targets: Vec<ConfiguredTarget> = Vec::new();
        for top_level_target in top_level_targets {
            let configuration = configuration_of(top_level_target);
            let compatibility = match Self::compatibility_with_target_environment(
                top_level_target,
                configuration.as_deref(),
            ) {
                Ok(c) => c,
                Err(e) => {
                    return Err(ViewCreationFailedException {
                        message: e.get_message().to_owned(),
                        failure_detail: e.get_failure_detail().clone(),
                    });
                }
            };
            if compatibility.is_compatible() {
                continue;
            }
            // PORT: unreachable until getMissingEnvironments is ported
            // (compatibility_with_target_environment fails loudly instead of returning an
            // incompatible result).
            bad_targets.push(top_level_target.clone());
        }
        Ok(bad_targets)
    }
}

// Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:147 TopLevelConstraintSemantics.getExceptionForExplicitlyRequestedIncompatibleTarget
fn get_exception_for_explicitly_requested_incompatible_target(
    configured_target: &ConfiguredTarget,
    underlying_target: &ConfiguredTarget,
) -> TargetCompatibilityCheckException {
    let target_incompatible_message =
        get_incompatible_message(configured_target, underlying_target);
    TargetCompatibilityCheckException::new(
        target_incompatible_message.clone(),
        FailureDetail {
            message: target_incompatible_message,
            category: Some(Category::Analysis(failure_details::Analysis {
                code: failure_details::analysis::Code::IncompatibleTargetRequested as i32,
            })),
        },
    )
}

// Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:159 TopLevelConstraintSemantics.getIncompatibleMessage
fn get_incompatible_message(
    configured_target: &ConfiguredTarget,
    underlying_target: &ConfiguredTarget,
) -> String {
    format!(
        "Target {} is incompatible and cannot be built, but was explicitly requested.{}",
        configured_target.get_original_label(),
        // We need access to the provider so we pass in the underlying target here that is
        // responsible for the incompatibility.
        report_on_incompatibility(underlying_target)
    )
}

/// Assembles the explanation for a platform incompatibility.
///
/// This is useful when trying to explain to the user why an explicitly requested target on the
/// command line is considered incompatible. The goal is to print out the dependency chain and the
/// constraint or `config_setting` that wasn't satisfied so that the user can immediately figure
/// out what happened.
///
/// `target`: the incompatible target that was explicitly requested on the command line. Returns
/// the verbose error message to show to the user.
// Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:284 TopLevelConstraintSemantics.reportOnIncompatibility
fn report_on_incompatibility(target: &ConfiguredTarget) -> String {
    let mut message = String::from("\nDependency chain:");
    let mut provider;

    // TODO(austinschuh): While the first error is helpful, reporting all the errors at once would
    // save the user bazel round trips.
    let mut target: ConfiguredTarget = target.clone();
    loop {
        let checksum = target
            .get_configuration_checksum()
            .unwrap_or_else(|| panic!("NullPointerException: getConfigurationChecksum()"));
        message.push_str(&format!(
            "\n    {} ({})",
            target.get_label(),
            // Java: `substring(0, 6)`.
            &checksum[..6]
        ));
        provider = target
            .get(&INCOMPATIBLE_PLATFORM_PROVIDER.key())
            .unwrap_or_else(|| panic!("NullPointerException: IncompatiblePlatformProvider"));
        let incompatible_platform_provider =
            downcast_info::<IncompatiblePlatformProvider>(&provider)
                .expect("IncompatiblePlatformProvider");
        match incompatible_platform_provider.targets_responsible_for_incompatibility() {
            None => break,
            Some(target_list) => {
                let next = target_list[0].clone();
                target = next;
            }
        }
    }

    let incompatible_platform_provider = downcast_info::<IncompatiblePlatformProvider>(&provider)
        .expect("IncompatiblePlatformProvider");
    message.push_str(&format!(
        "   <--{}",
        format_incompatibility_reasons(
            incompatible_platform_provider.target_platform(),
            incompatible_platform_provider.constraints_responsible_for_incompatibility(),
            incompatible_platform_provider.config_settings_responsible_for_incompatibility(),
        )
    ));
    message
}

// Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:316 TopLevelConstraintSemantics.formatIncompatibilityReasons
fn format_incompatibility_reasons(
    target_platform: Option<&Label>,
    constraints: Option<&[ConstraintValueInfo]>,
    config_settings: Option<&[Label]>,
) -> String {
    // StringJoiner(" and")
    let mut reasons: Vec<String> = Vec::new();
    if let Some(constraints) = constraints {
        reasons.push(format_constraint_incompatibility(
            target_platform,
            constraints,
        ));
    }
    if let Some(config_settings) = config_settings {
        reasons.push(format_config_setting_incompatibility(config_settings));
    }
    reasons.join(" and")
}

// Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:330 TopLevelConstraintSemantics.formatConstraintIncompatibility
fn format_constraint_incompatibility(
    target_platform: Option<&Label>,
    constraints: &[ConstraintValueInfo],
) -> String {
    let message = format!(
        " target platform ({}) didn't satisfy constraint",
        match target_platform {
            Some(label) => label.to_string(),
            // Java: String.format("%s", null).
            None => "null".to_owned(),
        }
    );
    if constraints.len() == 1 {
        return format!("{} {}", message, constraints[0].label());
    }
    format!(
        "{}s [{}]",
        message,
        constraints
            .iter()
            .map(|constraint_value_info| constraint_value_info.label().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

// Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:345 TopLevelConstraintSemantics.formatConfigSettingIncompatibility
fn format_config_setting_incompatibility(config_settings: &[Label]) -> String {
    if config_settings.len() == 1 {
        return format!(
            " target configuration didn't match config_setting {}",
            config_settings[0]
        );
    }
    format!(
        " target configuration didn't match config_settings [{}]",
        config_settings
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

#[cfg(test)]
mod tests {
    // PORT: TopLevelConstraintSemantics has no JUnit test at the pin; its behaviour is tested by
    // src/test/shell/integration/target_compatible_with_test.sh. The expected texts below are the
    // oracle's (Bazel 9.3.2) output for scripts/w4/scenarios/incompat-toplevel and
    // scripts/w4/scenarios/incompat-config-setting (the text after `<--`).
    use super::*;
    use crate::platform::constraint_setting_info::ConstraintSettingInfo;

    fn label(s: &str) -> Label {
        Label::parse_canonical_unchecked(s)
    }

    fn constraint(setting: &str, value: &str) -> ConstraintValueInfo {
        ConstraintValueInfo::create(ConstraintSettingInfo::create(label(setting)), label(value))
    }

    // `bazel build //pkg:bad`.
    #[test]
    fn format_incompatibility_reasons_one_constraint() {
        assert_eq!(
            format_incompatibility_reasons(
                Some(&label("@@platforms//host:host")),
                Some(&[constraint(
                    "@@platforms//:incompatible_setting",
                    "@@platforms//:incompatible"
                )]),
                None,
            ),
            " target platform (@@platforms//host:host) didn't satisfy constraint \
             @@platforms//:incompatible"
        );
    }

    // `bazel build //pkg:needs_two`.
    #[test]
    fn format_incompatibility_reasons_two_constraints() {
        assert_eq!(
            format_incompatibility_reasons(
                Some(&label("@@platforms//host:host")),
                Some(&[
                    constraint("//pkg:flavor", "//pkg:vanilla"),
                    constraint(
                        "@@platforms//:incompatible_setting",
                        "@@platforms//:incompatible"
                    ),
                ]),
                None,
            ),
            " target platform (@@platforms//host:host) didn't satisfy constraints \
             [//pkg:vanilla, @@platforms//:incompatible]"
        );
    }

    // `bazel build //pkg:cs_one` (scenario incompat-config-setting).
    #[test]
    fn format_incompatibility_reasons_one_config_setting() {
        assert_eq!(
            format_incompatibility_reasons(
                Some(&label("@@platforms//host:host")),
                None,
                Some(&[label("//pkg:is_opt")]),
            ),
            " target configuration didn't match config_setting //pkg:is_opt"
        );
    }

    // `bazel build //pkg:cs_two` and `//pkg:cs_mixed` (scenario incompat-config-setting).
    #[test]
    fn format_incompatibility_reasons_config_settings_and_constraint() {
        assert_eq!(
            format_incompatibility_reasons(
                Some(&label("@@platforms//host:host")),
                None,
                Some(&[label("//pkg:is_dbg"), label("//pkg:is_opt")]),
            ),
            " target configuration didn't match config_settings [//pkg:is_dbg, //pkg:is_opt]"
        );
        assert_eq!(
            format_incompatibility_reasons(
                Some(&label("@@platforms//host:host")),
                Some(&[constraint("//pkg:flavor", "//pkg:vanilla")]),
                Some(&[label("//pkg:is_opt")]),
            ),
            " target platform (@@platforms//host:host) didn't satisfy constraint //pkg:vanilla \
             and target configuration didn't match config_setting //pkg:is_opt"
        );
    }
}
