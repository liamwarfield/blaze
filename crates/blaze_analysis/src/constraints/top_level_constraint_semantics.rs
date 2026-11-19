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
use std::sync::Arc;

use blaze_packages::target::Target;
use blaze_packages::target::environment_group::EnvironmentGroup;

use crate::config::incompatible_platform_provider::{
    INCOMPATIBLE_PLATFORM_PROVIDER, IncompatiblePlatformProvider,
};
use crate::configured_target::ConfiguredTarget;
use crate::constraints::environment_collection::EnvironmentCollectionBuilder;
use crate::constraints::rule_context_constraint_semantics::RuleContextConstraintSemantics;
use crate::constraints::supported_environments::{
    RemovedEnvironmentCulprit, SupportedEnvironments,
};
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

/// An expected environment a top-level target does not support.
// Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:91 TopLevelConstraintSemantics.MissingEnvironment
#[derive(Clone, Debug)]
pub struct MissingEnvironment {
    environment: Label,
    /// If null, the top-level target just didn't declare a required environment. If not null,
    /// that means the declaration got "refined" away due to some select() somewhere in its
    /// deps. See ConstraintSemantics's documentation for an explanation of refinement.
    culprit: Option<RemovedEnvironmentCulprit>,
}

/// Tells the compatibility of a ConfiguredTarget with the target environment.
// Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:579 TopLevelConstraintSemantics.EnvironmentCompatibility
#[derive(Clone, Debug)]
pub struct EnvironmentCompatibility {
    is_compatible: bool,
    /// `ImmutableSet<MissingEnvironment>`: `MissingEnvironment` has identity equality, so the
    /// set keeps every element.
    severe_missing_environments: Option<Vec<MissingEnvironment>>,
}

impl EnvironmentCompatibility {
    // Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:582 TopLevelConstraintSemantics.EnvironmentCompatibility.compatible
    pub fn compatible() -> EnvironmentCompatibility {
        EnvironmentCompatibility {
            is_compatible: true,
            severe_missing_environments: None,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:587 TopLevelConstraintSemantics.EnvironmentCompatibility.nonSevereIncompatible
    pub fn non_severe_incompatible() -> EnvironmentCompatibility {
        EnvironmentCompatibility {
            is_compatible: false,
            severe_missing_environments: None,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:592 TopLevelConstraintSemantics.EnvironmentCompatibility.severeIncompatible
    pub fn severe_incompatible(
        severe_missing_environments: Vec<MissingEnvironment>,
    ) -> EnvironmentCompatibility {
        EnvironmentCompatibility {
            is_compatible: false,
            severe_missing_environments: Some(severe_missing_environments),
        }
    }

    pub fn is_compatible(&self) -> bool {
        self.is_compatible
    }

    pub fn severe_missing_environments(&self) -> Option<&[MissingEnvironment]> {
        self.severe_missing_environments.as_deref()
    }
}

/// What `TargetLookup.getTarget` throws (`NoSuchPackageException`, `NoSuchTargetException`):
/// the message and the failure detail of its `DetailedExitCode`.
#[derive(Clone, Debug)]
pub struct TargetLookupException {
    pub message: String,
    pub failure_detail: FailureDetail,
}

/// Looks up a `Target` by its label.
///
/// PORT: Java's `getTarget` returns the target, and `ConstraintSemantics.getEnvironmentGroup`
/// then reads the environment groups of the target's package
/// (`pkg.getTargets(EnvironmentGroup.class)`). Rust targets do not reach their package, so the
/// lookup returns the package's environment groups with the target.
// Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:633 TopLevelConstraintSemantics.TargetLookup
pub trait TargetLookup {
    /// `None` if the implementation involves a Skyframe lookup and the value is missing.
    fn get_target(
        &self,
        label: &Label,
    ) -> Result<Option<(Target, Vec<Arc<EnvironmentGroup>>)>, TargetLookupException>;
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
    /// Returns `None` if the `target_lookup` performs a Skyframe lookup and the value is
    /// missing.
    // Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:173 TopLevelConstraintSemantics.compatibilityWithTargetEnvironment
    pub fn compatibility_with_target_environment(
        configured_target: &ConfiguredTarget,
        build_configuration_value: Option<&BuildConfigurationValue>,
        target_lookup: &dyn TargetLookup,
        event_handler: &dyn ExtendedEventHandler,
    ) -> Result<Option<EnvironmentCompatibility>, TargetCompatibilityCheckException> {
        // TODO(bazel-team): support file targets (they should apply package-default
        // constraints).
        let Some(build_configuration_value) = build_configuration_value else {
            return Ok(Some(EnvironmentCompatibility::compatible()));
        };
        let core_options = crate::rule::config_util::core_options(build_configuration_value);
        if !core_options.enforce_constraints || core_options.target_environments.is_empty() {
            return Ok(Some(EnvironmentCompatibility::compatible()));
        }

        let target = match target_lookup.get_target(&configured_target.get_label()) {
            Ok(target) => target.expect(
                "NullPointerException: targetLookup.getTarget(configuredTarget.getLabel())",
            ),
            Err(e) => {
                event_handler.handle(&Event::error(format!(
                    "Unable to get target from package when checking environment restrictions. {}",
                    e.message
                )));
                return Ok(Some(EnvironmentCompatibility::compatible()));
            }
        };

        match target.0.get_associated_rule() {
            Some(rule) if rule.get_rule_class_object().supports_constraint_checking() => {}
            _ => return Ok(Some(EnvironmentCompatibility::compatible())),
        }

        // Check explicitly expected environments.
        let target_environments: Vec<Label> = core_options
            .target_environments
            .iter()
            .map(|l| l.get().clone())
            .collect();
        let Some(severe_missing_environments) =
            get_missing_environments(configured_target, &target_environments, target_lookup)?
        else {
            // Missing value.
            return Ok(None);
        };

        if !severe_missing_environments.is_empty() {
            return Ok(Some(EnvironmentCompatibility::severe_incompatible(
                severe_missing_environments,
            )));
        }

        Ok(Some(EnvironmentCompatibility::compatible()))
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
        target_lookup: &dyn TargetLookup,
        event_handler: &dyn ExtendedEventHandler,
    ) -> Result<Vec<ConfiguredTarget>, ViewCreationFailedException> {
        let mut bad_targets: Vec<ConfiguredTarget> = Vec::new();
        // Maps targets that are missing *explicitly* required environments to the set of
        // environments they're missing. These targets trigger a ViewCreationFailedException,
        // which halts the build. Targets with missing *implicitly* required environments don't
        // belong here, since the build continues while skipping them.
        let mut exception_inducing_targets: Vec<(ConfiguredTarget, Vec<MissingEnvironment>)> =
            Vec::new();
        for top_level_target in top_level_targets {
            let configuration = configuration_of(top_level_target);
            let compatibility = match Self::compatibility_with_target_environment(
                top_level_target,
                configuration.as_deref(),
                target_lookup,
                event_handler,
            ) {
                Ok(c) => c.expect("NullPointerException: compatibilityWithTargetEnvironment"),
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
            if let Some(missing) = compatibility.severe_missing_environments() {
                // ArrayListMultimap.putAll: an empty collection adds no key.
                if !missing.is_empty() {
                    exception_inducing_targets.push((top_level_target.clone(), missing.to_vec()));
                }
            }
            // ImmutableSet.Builder.add
            if !bad_targets
                .iter()
                .any(|t| t.identity() == top_level_target.identity())
            {
                bad_targets.push(top_level_target.clone());
            }
        }

        if !exception_inducing_targets.is_empty() {
            let bad_targets_user_message =
                get_bad_targets_user_message(&exception_inducing_targets);
            return Err(ViewCreationFailedException {
                message: bad_targets_user_message.clone(),
                failure_detail: FailureDetail {
                    message: bad_targets_user_message,
                    category: Some(Category::Analysis(failure_details::Analysis {
                        code: failure_details::analysis::Code::TargetsMissingEnvironments as i32,
                    })),
                },
            });
        }
        Ok(bad_targets)
    }
}

/// Returns the environment group that owns the given environment. Both must belong to the same
/// package.
// Java: com/google/devtools/build/lib/analysis/constraints/ConstraintSemantics.java:136 ConstraintSemantics.getEnvironmentGroup
fn get_environment_group(
    env_target: &Target,
    package_environment_groups: &[Arc<EnvironmentGroup>],
) -> Result<Arc<EnvironmentGroup>, TargetLookupException> {
    let is_environment_rule = env_target.as_rule().is_some_and(|rule| {
        rule.get_rule_class() == crate::platform::constraint_constants::ENVIRONMENT_RULE
    });
    if !is_environment_rule {
        return Err(create_environment_lookup_exception(
            format!(
                "{} is not a valid environment definition",
                env_target.get_label()
            ),
            failure_details::analysis::Code::InvalidEnvironment,
        ));
    }
    // TODO(https://github.com/bazelbuild/bazel/issues/23852): support package pieces.
    for group in package_environment_groups {
        if group.get_environments().contains(env_target.get_label()) {
            return Ok(group.clone());
        }
    }
    Err(create_environment_lookup_exception(
        format!(
            "cannot find the group for environment {}",
            env_target.get_label()
        ),
        failure_details::analysis::Code::EnvironmentMissingFromGroups,
    ))
}

// Java: com/google/devtools/build/lib/analysis/constraints/ConstraintSemantics.java:214 ConstraintSemantics.createEnvironmentLookupException
fn create_environment_lookup_exception(
    message: String,
    code: failure_details::analysis::Code,
) -> TargetLookupException {
    TargetLookupException {
        message: message.clone(),
        failure_detail: FailureDetail {
            message,
            category: Some(Category::Analysis(failure_details::Analysis {
                code: code as i32,
            })),
        },
    }
}

/// The target whose providers describe `configured_target`'s environments: the alias-resolved
/// target, or for an output file its generating rule.
fn target_with_provider(configured_target: &ConfiguredTarget) -> ConfiguredTarget {
    // Dereference any aliases that might be present.
    let actual = configured_target.get_actual();
    match &actual {
        ConfiguredTarget::OutputFile(output_file_configured_target) => {
            ConfiguredTarget::Rule(output_file_configured_target.get_generating_rule().clone())
        }
        _ => actual,
    }
}

/// Returns the expected environments that the given top-level target doesn't support.
///
/// `None` where Java returns null (a missing Skyframe value).
// Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:448 TopLevelConstraintSemantics.getMissingEnvironments
fn get_missing_environments(
    top_level_target: &ConfiguredTarget,
    expected_environment_labels: &[Label],
    target_lookup: &dyn TargetLookup,
) -> Result<Option<Vec<MissingEnvironment>>, TargetCompatibilityCheckException> {
    // Convert expected environment labels to actual environments.
    let mut expected_environments_builder = EnvironmentCollectionBuilder::new();
    for env_label in expected_environment_labels {
        let invalid = |e: TargetLookupException| {
            TargetCompatibilityCheckException::new(
                format!("invalid target environment: {}", e.message),
                e.failure_detail,
            )
        };
        let Some((env, groups)) = target_lookup.get_target(env_label).map_err(invalid)? else {
            // Missing value.
            return Ok(None);
        };
        let group = get_environment_group(&env, &groups).map_err(invalid)?;
        expected_environments_builder.put(
            &Arc::new(group.get_environment_labels().clone()),
            env_label.clone(),
        );
    }
    let expected_environments = expected_environments_builder.build();

    // Now check the target against expected environments.
    let as_provider = target_with_provider(top_level_target);
    let provider = as_provider
        .get_provider::<SupportedEnvironments>()
        .unwrap_or_else(|| panic!("VerifyException: no SupportedEnvironmentsProvider"));
    let mut ans: Vec<MissingEnvironment> = Vec::new();
    for unsupported_env in RuleContextConstraintSemantics::get_unsupported_environments(
        provider.get_refined_environments(),
        &expected_environments,
    ) {
        // We apply this filter because the target might also not support default environments
        // in other environment groups. We don't care about those. We only care about the
        // environments explicitly referenced.
        if !expected_environment_labels.contains(&unsupported_env) {
            continue;
        }

        let mut env_and_fulfillers: Vec<Label> = vec![unsupported_env.clone()];
        for env_group in provider.get_static_environments().get_groups() {
            env_and_fulfillers.extend(
                env_group
                    .get_fulfillers(&unsupported_env)
                    .to_list()
                    .iter()
                    .cloned(),
            );
        }
        let mut culprit: Option<RemovedEnvironmentCulprit> = None;
        let mut i = 0;
        while i < env_and_fulfillers.len() && culprit.is_none() {
            culprit = provider
                .get_removed_environment_culprit(&env_and_fulfillers[i])
                .cloned();
            i += 1;
        }
        // culprit could still be null here. See MissingEnvironment class comments for
        // implications.
        ans.push(MissingEnvironment {
            environment: unsupported_env,
            culprit,
        });
    }
    Ok(Some(ans))
}

/// Prepares a user-friendly error message for a list of targets missing support for required
/// environments.
// Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:516 TopLevelConstraintSemantics.getBadTargetsUserMessage
fn get_bad_targets_user_message(
    bad_targets: &[(ConfiguredTarget, Vec<MissingEnvironment>)],
) -> String {
    let mut msg: Vec<String> = vec!["This is a restricted-environment build.".to_string()];
    for (target, missing) in bad_targets {
        msg.push(get_error_message_for_target(target, missing));
    }
    msg.push(" ".to_string());
    msg.join("\n")
}

// Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:528 TopLevelConstraintSemantics.getErrorMessageForTarget
pub fn get_error_message_for_target(
    configured_target: &ConfiguredTarget,
    missing_environments: &[MissingEnvironment],
) -> String {
    let mut msg: Vec<String> = Vec::new();
    let target_with_provider = target_with_provider(configured_target);
    let supported_environments = target_with_provider
        .get_provider::<SupportedEnvironments>()
        .expect("NullPointerException: SupportedEnvironmentsProvider");
    let declared_envs = supported_environments
        .get_static_environments()
        .get_environments()
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    msg.push(" ".to_string());
    msg.push(format!(
        "{} declares compatibility with:",
        configured_target.get_label()
    ));
    msg.push(format!("  [{}]", declared_envs));
    msg.push("but does not support:".to_string());
    let mut is_first = true;
    let mut last_entry_was_multiline = false;
    for missing_environment in missing_environments {
        match &missing_environment.culprit {
            None => {
                // The target didn't declare support for this environment.
                if last_entry_was_multiline {
                    // Pretty-format: if the last environment message was multi-line, make it
                    // clear this one is a different entry. But we don't want to do that if all
                    // entries are single-line because that would be pointlessly long.
                    msg.push(" ".to_string());
                }
                msg.push(format!("  {}", missing_environment.environment));
                last_entry_was_multiline = false;
            }
            Some(culprit) => {
                // The target declared support, but it was refined out by a select() somewhere
                // in its transitive deps.
                if !is_first {
                    msg.push(" ".to_string()); // Pretty-format for clarity.
                }
                msg.push(
                    RuleContextConstraintSemantics::get_missing_environment_culprit_message(
                        &configured_target.get_label(),
                        &missing_environment.environment,
                        culprit,
                    ),
                );
                last_entry_was_multiline = true;
            }
        }
        is_first = false;
    }
    msg.join("\n")
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
