//! The `RuleContext`-facing half of `analysis/RunfilesSupport.java`: `withExecutable`,
//! `computeArgs`, `computeActionEnvironment` and the rule values `create` reads.
//! `blaze_actions::analysis::runfiles_support::RunfilesSupport` ports the rest; this module
//! implements its `RunfilesActionConstructionContext` over `RuleContext`.

use std::sync::Arc;

use blaze_actions::analysis::actions::symlink_tree_action::RunfileSymlinksMode;
use blaze_actions::analysis::runfiles::Runfiles;
use blaze_actions::analysis::runfiles_support::{
    RunfilesActionConstructionContext, RunfilesSupport, RunfilesSupportRuleValues,
};
use blaze_actions::{ActionEnvironment, ActionOwner, ActionRef, Artifact, ArtifactRoot};
use blaze_optdefs::fragment::run_under::RunUnder;
use blaze_packages::attribute::AttributeMap;
use blaze_packages::target::{RuleOrMacroInstance, target_utils};
use blaze_packages::types::Type;
use blaze_vfs::PathFragment;
use starlark::values::OwnedFrozenValue;

use crate::configured_rule_class_provider::CreateConfiguredTargetError;
use crate::info::downcast_info;
use crate::providers::file_provider::{FileProvider, RunfilesProvider};
use crate::providers::run_environment_info::RunEnvironmentInfo;
use crate::rule::action_environments::to_actions_env;
use crate::rule::config_util;
use crate::rule::rule_context::RuleContext;
use crate::test::test_action_builder::TestActionBuilder;

impl RunfilesActionConstructionContext for RuleContext {
    fn get_action_owner(&self) -> ActionOwner {
        RuleContext::get_action_owner(self)
    }

    fn get_bin_directory(&self) -> ArtifactRoot {
        RuleContext::get_bin_directory(self)
    }

    fn get_derived_artifact(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
    ) -> Artifact {
        RuleContext::get_derived_artifact(self, root_relative_path, root)
    }

    fn get_runfiles_artifact(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
    ) -> Artifact {
        self.get_analysis_environment()
            .get_runfiles_artifact(root_relative_path, root)
    }

    fn register_action(&self, action: ActionRef) {
        RuleContext::register_action(self, action);
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:867 BuildConfigurationValue.getRunfileSymlinksMode
    fn get_runfile_symlinks_mode(&self) -> RunfileSymlinksMode {
        match self.get_configuration().get_runfile_symlinks_mode() {
            blaze_build_config::build_configuration_value::RunfileSymlinksMode::SKIP => {
                RunfileSymlinksMode::SKIP
            }
            blaze_build_config::build_configuration_value::RunfileSymlinksMode::CREATE => {
                RunfileSymlinksMode::CREATE
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:689 BuildConfigurationValue.buildRunfileManifests
    fn build_runfile_manifests(&self) -> bool {
        config_util::core_options(self.get_configuration()).build_runfile_manifests
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:694 BuildConfigurationValue.buildRunfileLinks
    fn build_runfile_links(&self) -> bool {
        let options = config_util::core_options(self.get_configuration());
        options.build_runfile_manifests && options.build_runfile_links
    }

    fn is_sibling_repository_layout(&self) -> bool {
        config_util::is_sibling_repository_layout(self.get_configuration())
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildConfigurationValue.java:894 BuildConfigurationValue.remotableSourceManifestActions
    fn remotable_source_manifest_actions(&self) -> bool {
        config_util::core_options(self.get_configuration()).remotable_source_manifest_actions
    }

    fn get_configuration_action_environment(&self) -> ActionEnvironment {
        to_actions_env(self.get_configuration().get_action_environment())
    }

    fn get_configuration_workspace_name(&self) -> String {
        self.get_configuration().get_workspace_name().to_owned()
    }
}

/// Creates the RunfilesSupport helper class for an executable rule.
///
/// `run_environment_info` is a `RunEnvironmentInfo` info, if the rule returned one.
// Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:571 RunfilesSupport.withExecutable
pub fn with_executable(
    rule_context: &RuleContext,
    runfiles: Runfiles,
    executable: Artifact,
    run_environment_info: Option<&OwnedFrozenValue>,
) -> Result<RunfilesSupport, CreateConfiguredTargetError> {
    let run_environment_info = run_environment_info
        .map(|v| downcast_info::<RunEnvironmentInfo>(v).expect("RunEnvironmentInfo"));
    let args = compute_args(rule_context);
    let action_environment = compute_action_environment(rule_context, run_environment_info);
    let rule_values = rule_values(rule_context);
    Ok(RunfilesSupport::with_executable(
        rule_context,
        &rule_values,
        runfiles,
        executable,
        args,
        action_environment,
    ))
}

/// The values `RunfilesSupport.create` reads from the rule context.
// Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:139 RunfilesSupport.create
pub fn rule_values(rule_context: &RuleContext) -> RunfilesSupportRuleValues {
    let is_test_rule = target_utils::is_test_rule(rule_context.get_target());
    // Adding run_under target to the runfiles manifest so it would become part of runfiles tree
    // and would be executable everywhere.
    let run_under_runfiles = match config_util::get_run_under(rule_context.get_configuration()) {
        Some(RunUnder::LabelRunUnder { .. }) if is_test_rule => {
            let run_under_target = rule_context
                .get_run_under_prerequisite()
                .expect("NullPointerException: run_under prerequisite");
            // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:371 RunfilesSupport.getRunfiles
            Some(RunfilesSupport::get_runfiles_of_target(
                crate::configuredtargets::get_provider::<RunfilesProvider>(&run_under_target)
                    .map(|p| p.get_default_runfiles().clone()),
                || {
                    crate::configuredtargets::get_provider::<FileProvider>(&run_under_target)
                        .expect("FileProvider")
                        .get_files_to_build()
                        .clone()
                },
                rule_context.get_workspace_name(),
            ))
        }
        _ => None,
    };
    RunfilesSupportRuleValues {
        workspace_name: rule_context.get_workspace_name().to_owned(),
        // Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:530 RunfilesSupport.cacheRunfilesMappings
        cache_runfiles_mappings: RunfilesSupport::cache_runfiles_mappings(
            is_test_rule,
            || TestActionBuilder::get_runs_per_test(rule_context),
            || TestActionBuilder::get_shard_count(rule_context),
        ),
        run_under_runfiles,
        transitive_packages_for_runfile_repo_mapping_manifest: rule_context
            .get_transitive_packages_for_runfile_repo_mapping_manifest()
            .cloned(),
        compact_repo_mapping: config_util::core_options(rule_context.get_configuration())
            .compact_repo_mapping,
    }
}

// Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:598 RunfilesSupport.computeArgs
pub fn compute_args(
    rule_context: &RuleContext,
) -> blaze_actions::actions::command_line::CommandLineRef {
    RunfilesSupport::compute_args(
        rule_context
            .get_rule()
            .is_attr_defined("args", Type::StringList),
        || {
            rule_context
                .get_expander()
                .with_data_locations()
                .tokenized_attribute("args")
        },
    )
}

// Java: com/google/devtools/build/lib/analysis/RunfilesSupport.java:618 RunfilesSupport.computeActionEnvironment
pub fn compute_action_environment(
    rule_context: &RuleContext,
    run_environment_info: Option<&RunEnvironmentInfo>,
) -> ActionEnvironment {
    let rule = rule_context.get_rule();
    RunfilesSupport::compute_action_environment(
        run_environment_info.map(|r| {
            (
                r.get_environment().clone(),
                r.get_inherited_environment().to_vec(),
            )
        }),
        rule.get_rule_class_object()
            .get_rule_definition_environment_label()
            .is_none(),
        rule.is_attr_defined("env", Type::StringDict),
        rule.is_attr_defined("env_inherit", Type::StringList),
        || {
            let expander = rule_context.get_expander().with_data_locations();
            rule_context
                .attributes()
                .get("env", Type::StringDict)
                .and_then(|v| v.as_string_dict().cloned())
                .unwrap_or_default()
                .into_iter()
                .map(|(k, v)| {
                    let expanded = expander.expand(Some("env"), &v);
                    (k, expanded)
                })
                .collect()
        },
        || {
            rule_context
                .attributes()
                .get("env_inherit", Type::StringList)
                .and_then(|v| v.as_string_list().map(|s| s.to_vec()))
                .unwrap_or_default()
        },
    )
}

/// Keeps `Arc` in scope for callers that share the result.
pub type SharedRunfilesSupport = Arc<RunfilesSupport>;
