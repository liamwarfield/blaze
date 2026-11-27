//! Port of `skyframe/actiongraph/v2/ActionGraphDump.java` and the helpers of
//! `query2/aquery/AqueryUtils.java` it uses.

use std::collections::HashMap;

use blaze_collect::command_line_item::InterruptedException;
use blaze_collect::{NestedSet, NestedSetBuilder};
use blaze_protos::analysis;
use indexmap::IndexMap;

use crate::actions::action::{Action, CommandLineError};
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::artifact::Artifact;

use super::aquery_output_handler::AqueryOutputHandler;
use super::known::{
    KnownArtifacts, KnownAspectDescriptors, KnownConfigurations, KnownNestedSets, KnownTargets,
};

/// The functions of `AqueryActionFilter` (`mnemonic(...)`, `inputs(...)`, `outputs(...)`).
// Java: com/google/devtools/build/lib/query2/aquery/AqueryActionFilter.java AqueryActionFilter
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AqueryFilterFunction {
    MNEMONIC,
    INPUTS,
    OUTPUTS,
}

/// `AqueryActionFilter`: patterns per filter function.
///
/// PORT: the filter is built by the aquery environment (`blaze_query`); it implements this trait.
pub trait AqueryActionFilter {
    fn has_filter_for_function(&self, function: AqueryFilterFunction) -> bool;
    fn matches_all_patterns_for_function(
        &self,
        function: AqueryFilterFunction,
        value: &str,
    ) -> bool;
}

/// Errors of `dumpSingleAction`.
#[derive(Debug, thiserror::Error)]
pub enum ActionGraphDumpError {
    #[error("{0}")]
    CommandLineExpansion(String),
    #[error("InterruptedException")]
    Interrupted,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// `TemplateExpansionException("Failed to expand template", e)`.
    #[error("Failed to expand template")]
    TemplateExpansion(String),
}

impl From<CommandLineError> for ActionGraphDumpError {
    fn from(e: CommandLineError) -> Self {
        match e {
            CommandLineError::CommandLineExpansion(e) => {
                ActionGraphDumpError::CommandLineExpansion(e.message)
            }
            CommandLineError::Interrupted(_) => ActionGraphDumpError::Interrupted,
        }
    }
}

impl From<InterruptedException> for ActionGraphDumpError {
    fn from(_: InterruptedException) -> Self {
        ActionGraphDumpError::Interrupted
    }
}

/// Returns the inputs aquery reports for an action.
// Java: com/google/devtools/build/lib/query2/aquery/AqueryUtils.java:45 AqueryUtils.getActionInputs
pub fn get_action_inputs(action: &dyn Action, include_pruned_inputs: bool) -> NestedSet<Artifact> {
    if include_pruned_inputs || (action.is_action_execution_metadata() && !action.inputs_known()) {
        // getInputs() is potentially missing inputs that will be added by discovery (if the
        // action hasn't yet executed) and inputs that have been removed by discovery (if the
        // action has already executed). Instead, assemble the inputs from getOriginalInputs() and
        // getSchedulingDependencies(), which also include those added or removed by discovery.
        let mut b = NestedSetBuilder::stable_order();
        b.add_transitive(&action.get_original_inputs());
        b.add_transitive(&action.get_scheduling_dependencies());
        return b.build();
    }
    action.get_inputs()
}

/// Return true if the given `action` matches the filters specified in `action_filters`.
// Java: com/google/devtools/build/lib/query2/aquery/AqueryUtils.java:70 AqueryUtils.matchesAqueryFilters
pub fn matches_aquery_filters(
    action: &dyn Action,
    action_filters: &dyn AqueryActionFilter,
    include_pruned_inputs: bool,
) -> bool {
    let inputs = get_action_inputs(action, include_pruned_inputs);
    let outputs = action.get_outputs();
    let mnemonic = action.get_mnemonic();

    if action_filters.has_filter_for_function(AqueryFilterFunction::MNEMONIC) {
        if !action_filters
            .matches_all_patterns_for_function(AqueryFilterFunction::MNEMONIC, &mnemonic)
        {
            return false;
        }
    }

    if action_filters.has_filter_for_function(AqueryFilterFunction::INPUTS) {
        let contains_file = inputs.to_list().iter().any(|artifact| {
            action_filters.matches_all_patterns_for_function(
                AqueryFilterFunction::INPUTS,
                artifact.get_exec_path_string(),
            )
        });
        if !contains_file {
            return false;
        }
    }

    if action_filters.has_filter_for_function(AqueryFilterFunction::OUTPUTS) {
        let contains_file = outputs.iter().any(|artifact| {
            action_filters.matches_all_patterns_for_function(
                AqueryFilterFunction::OUTPUTS,
                artifact.get_exec_path_string(),
            )
        });
        return contains_file;
    }

    true
}

/// What aquery needs of the (actual) configured target owning the actions: its label string and
/// `RuleConfiguredTarget.getRuleClassString()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DumpedTarget {
    pub label: String,
    pub rule_class_string: Option<String>,
}

/// Encapsulates necessary functionality to dump the current skyframe state of the action graph to
/// proto format.
// Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/ActionGraphDump.java:63 ActionGraphDump
pub struct ActionGraphDump<'h> {
    action_key_context: ActionKeyContext,
    action_graph_targets: Vec<String>,
    known_artifacts: KnownArtifacts,
    known_configurations: KnownConfigurations,
    known_nested_sets: KnownNestedSets,
    known_aspect_descriptors: KnownAspectDescriptors,
    known_targets: KnownTargets,
    action_filters: Option<&'h dyn AqueryActionFilter>,
    include_action_cmd_line: bool,
    include_artifacts: bool,
    include_pruned_inputs: bool,
    include_param_files: bool,
    include_file_write_contents: bool,
    aquery_output_handler: &'h mut dyn AqueryOutputHandler,
    param_file_name_to_content_map: Option<HashMap<String, Vec<String>>>,
}

impl<'h> ActionGraphDump<'h> {
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/ActionGraphDump.java:82 ActionGraphDump.<init>
    pub fn new(
        include_action_cmd_line: bool,
        include_artifacts: bool,
        include_pruned_inputs: bool,
        action_filters: Option<&'h dyn AqueryActionFilter>,
        include_param_files: bool,
        include_file_write_contents: bool,
        aquery_output_handler: &'h mut dyn AqueryOutputHandler,
    ) -> Self {
        ActionGraphDump::with_targets(
            vec!["...".to_string()],
            include_action_cmd_line,
            include_artifacts,
            include_pruned_inputs,
            action_filters,
            include_param_files,
            include_file_write_contents,
            aquery_output_handler,
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/ActionGraphDump.java:82 ActionGraphDump.<init>
    pub fn with_targets(
        action_graph_targets: Vec<String>,
        include_action_cmd_line: bool,
        include_artifacts: bool,
        include_pruned_inputs: bool,
        action_filters: Option<&'h dyn AqueryActionFilter>,
        include_param_files: bool,
        include_file_write_contents: bool,
        aquery_output_handler: &'h mut dyn AqueryOutputHandler,
    ) -> Self {
        let mut targets = Vec::new();
        for t in action_graph_targets {
            if !targets.contains(&t) {
                targets.push(t);
            }
        }
        ActionGraphDump {
            action_key_context: ActionKeyContext::new(),
            action_graph_targets: targets,
            known_artifacts: KnownArtifacts::new(),
            known_configurations: KnownConfigurations::new(),
            known_nested_sets: KnownNestedSets::new(),
            known_aspect_descriptors: KnownAspectDescriptors::new(),
            known_targets: KnownTargets::new(),
            action_filters,
            include_action_cmd_line,
            include_artifacts,
            include_pruned_inputs,
            include_param_files,
            include_file_write_contents,
            aquery_output_handler,
            param_file_name_to_content_map: None,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/ActionGraphDump.java:131 ActionGraphDump.getActionKeyContext
    pub fn get_action_key_context(&self) -> &ActionKeyContext {
        &self.action_key_context
    }

    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/ActionGraphDump.java:135 ActionGraphDump.includeInActionGraph
    fn include_in_action_graph(&self, label_string: &str) -> bool {
        if self.action_graph_targets.len() == 1 && self.action_graph_targets[0] == "..." {
            return true;
        }
        self.action_graph_targets.iter().any(|t| t == label_string)
    }

    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/ActionGraphDump.java:143 ActionGraphDump.dumpSingleAction
    fn dump_single_action(
        &mut self,
        configured_target: &DumpedTarget,
        action: &dyn Action,
    ) -> Result<(), ActionGraphDumpError> {
        // Store the content of param files.
        if self.include_param_files {
            if let Some(param_file_args) = dump_hooks::parameter_file_write_arguments(action)? {
                let param_file_exec_path = action
                    .get_primary_output()
                    .get_exec_path_string()
                    .to_string();
                self.param_file_name_to_content_map
                    .get_or_insert_with(HashMap::new)
                    .insert(param_file_exec_path, param_file_args);
            }
        }

        if let Some(filters) = self.action_filters {
            if !matches_aquery_filters(action, filters, self.include_pruned_inputs) {
                return Ok(());
            }
        }

        // Dereferencing aliases: the caller passes the actual target
        // (`configuredTarget.getActual()`), which must be a RuleConfiguredTarget.
        let out = &mut *self.aquery_output_handler;
        let mut action_builder = analysis::Action {
            mnemonic: action.get_mnemonic(),
            target_id: self.known_targets.data_to_id_and_stream_output_proto(
                (
                    &configured_target.label,
                    configured_target.rule_class_string.as_deref(),
                ),
                out,
            ),
            ..Default::default()
        };

        if action.is_action_execution_metadata() {
            action_builder.action_key = action.get_key(&self.action_key_context, None)?;
            action_builder.discovers_inputs = action.discovers_inputs();
        }

        // store environment
        if action.as_command_action().is_some() {
            let fixed_environment = action
                .get_effective_environment(&IndexMap::new(), &crate::actions::path_mapper::NOOP)
                .map_err(|e| ActionGraphDumpError::CommandLineExpansion(e.message))?;
            for (key, value) in fixed_environment {
                action_builder
                    .environment_variables
                    .push(analysis::KeyValuePair { key, value });
            }
        }

        if self.include_action_cmd_line {
            if let Some(command_action) = action.as_command_action() {
                action_builder.arguments = command_action.get_arguments()?;
            }
        }

        // Include the content of param files in output.
        if let Some(provider) = action.as_file_contents_provider() {
            action_builder.is_executable = provider.make_executable();
            if self.include_file_write_contents {
                action_builder.file_contents = provider.get_file_contents()?;
            }
        }

        if let Some(target) = dump_hooks::unresolved_symlink_target(action) {
            action_builder.unresolved_symlink_target = target;
        }

        if self.include_param_files {
            // Assumption: if an Action takes a params file as an input, it will be used
            // to provide params to the command.
            let inputs = get_action_inputs(action, self.include_pruned_inputs);
            for input in inputs.to_list().iter() {
                let input_file_exec_path = input.get_exec_path_string();
                if let Some(args) = self
                    .param_file_name_to_content_map
                    .as_ref()
                    .and_then(|m| m.get(input_file_exec_path))
                {
                    action_builder.param_files.push(analysis::ParamFile {
                        exec_path: input_file_exec_path.to_string(),
                        arguments: args.clone(),
                    });
                }
            }
        }

        let execution_info = action.get_execution_info();
        for (key, value) in execution_info {
            action_builder
                .execution_info
                .push(analysis::KeyValuePair { key, value });
        }

        let action_owner = action.get_owner();
        if let Some(event) = action_owner.get_build_configuration_event() {
            action_builder.configuration_id = self
                .known_configurations
                .data_to_id_and_stream_output_proto(event, out);
        } else {
            // PORT: Java passes a null event to the cache (NullPointerException); configured
            // targets always have one.
            panic!("NullPointerException: action owner has no BuildConfigurationEvent");
        }
        if let Some(platform) = action_owner.get_execution_platform() {
            action_builder.execution_platform = platform.label().to_string();
        }

        // Store aspects.
        // Iterate through the aspect path and dump the aspect descriptors.
        // In the case of aspect-on-aspect, AspectDescriptors are listed in topological order
        // of the configured target graph.
        // e.g. [A, B] would imply that aspect A is applied on top of aspect B.
        for aspect_descriptor in action_owner.get_aspect_descriptors().iter().rev() {
            let id = self
                .known_aspect_descriptors
                .data_to_id_and_stream_output_proto(aspect_descriptor, out);
            action_builder.aspect_descriptor_ids.push(id);
        }

        if self.include_artifacts {
            // Store inputs
            let inputs = get_action_inputs(action, self.include_pruned_inputs);
            if !inputs.is_empty() {
                let id = self.known_nested_sets.data_to_id_and_stream_output_proto(
                    &inputs,
                    &mut self.known_artifacts,
                    out,
                );
                action_builder.input_dep_set_ids.push(id);
            }

            // store outputs
            for artifact in action.get_outputs() {
                let id = self
                    .known_artifacts
                    .data_to_id_and_stream_output_proto(&artifact, out);
                action_builder.output_ids.push(id);
            }

            action_builder.primary_output_id = self
                .known_artifacts
                .data_to_id_and_stream_output_proto(&action.get_primary_output(), out);
        }

        if let Some((template_content, substitutions)) = dump_hooks::template_expansion(action)? {
            action_builder.template_content = template_content;
            for (key, value) in substitutions {
                action_builder
                    .substitutions
                    .push(analysis::KeyValuePair { key, value });
            }
        }

        out.output_action(action_builder);
        Ok(())
    }

    /// Dumps the actions of an aspect applied to a configured target.
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/ActionGraphDump.java:298 ActionGraphDump.dumpAspect
    pub fn dump_aspect(
        &mut self,
        aspect_actions: Option<&[std::sync::Arc<dyn Action>]>,
        configured_target: &DumpedTarget,
    ) -> Result<(), ActionGraphDumpError> {
        let Some(actions) = aspect_actions else {
            return Ok(());
        };
        if !self.include_in_action_graph(&configured_target.label) {
            return Ok(());
        }
        for action in actions {
            self.dump_single_action(configured_target, action.as_ref())?;
        }
        Ok(())
    }

    /// Dumps the actions of a rule configured target.
    // Java: com/google/devtools/build/lib/skyframe/actiongraph/v2/ActionGraphDump.java:319 ActionGraphDump.dumpConfiguredTarget
    pub fn dump_configured_target(
        &mut self,
        configured_target: &DumpedTarget,
        actions: &[std::sync::Arc<dyn Action>],
    ) -> Result<(), ActionGraphDumpError> {
        if !self.include_in_action_graph(&configured_target.label) {
            return Ok(());
        }
        for action in actions {
            self.dump_single_action(configured_target, action.as_ref())?;
        }
        Ok(())
    }
}

/// The class-specific `instanceof` branches of `dumpSingleAction`, through the [`Action`] hooks.
mod dump_hooks {
    use super::ActionGraphDumpError;
    use crate::actions::action::Action;

    /// `action instanceof ParameterFileWriteAction`: its arguments.
    pub fn parameter_file_write_arguments(
        action: &dyn Action,
    ) -> Result<Option<Vec<String>>, ActionGraphDumpError> {
        match action.aquery_parameter_file_arguments() {
            None => Ok(None),
            Some(r) => Ok(Some(r?)),
        }
    }

    /// `action instanceof UnresolvedSymlinkAction`: its target.
    pub fn unresolved_symlink_target(action: &dyn Action) -> Option<String> {
        action.aquery_unresolved_symlink_target()
    }

    /// `action instanceof TemplateExpansionAction`: template content and substitutions.
    pub fn template_expansion(
        action: &dyn Action,
    ) -> Result<Option<(String, Vec<(String, String)>)>, ActionGraphDumpError> {
        match action.aquery_template_expansion() {
            None => Ok(None),
            Some(Ok(v)) => Ok(Some(v)),
            Some(Err(e)) => Err(ActionGraphDumpError::TemplateExpansion(e)),
        }
    }
}
