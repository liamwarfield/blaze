//! Port of `analysis/extra/ExtraActionSpec.java`.

use std::sync::Arc;

use blaze_actions::actions::action_owner::ActionOwner;
use blaze_actions::{ActionKeyContext, ActionRef, Artifact};
use blaze_cmdline::Label;
use blaze_collect::command_line_item::InterruptedException;
use blaze_collect::{NestedSet, NestedSetBuilder};
use blaze_util::Fingerprint;
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

use super::extra_action::ExtraAction;
use super::extra_action_info_file_write_action::ExtraActionInfoFileWriteAction;
use crate::platform::constraint_constants::get_os_from_constraints_or_host;
use crate::provider_map::TransitiveInfoProvider;
use crate::rule::RuleContext;
use crate::rule::action_environments::to_actions_env;
use crate::rule::command_helper::{CommandHelper, build_bash_command_constructor};
use crate::rule::sh_toolchain;

/// The specification for a particular extra action type.
///
/// PORT: a handle on shared data: clones are the same Java object (`ExtraActionSpec` has
/// identity equality; see [`ExtraActionSpec::same`]).
// Java: com/google/devtools/build/lib/analysis/extra/ExtraActionSpec.java:47 ExtraActionSpec
#[derive(Clone, Debug)]
pub struct ExtraActionSpec(Arc<ExtraActionSpecData>);

/// The fields of [`ExtraActionSpec`].
#[derive(Debug)]
pub struct ExtraActionSpecData {
    resolved_tools: NestedSet<Artifact>,
    resolved_data: Vec<Artifact>,
    output_templates: Vec<String>,
    /// `ImmutableMap.copyOf(executionInfo)` (the order of `TargetUtils.getExecutionInfo`).
    execution_info: IndexMap<String, String>,
    command: String,
    requires_action_output: bool,
    label: Label,
}

impl ExtraActionSpec {
    // Java: com/google/devtools/build/lib/analysis/extra/ExtraActionSpec.java:56 ExtraActionSpec.<init>
    pub fn new(
        resolved_tools: NestedSet<Artifact>,
        resolved_data: Vec<Artifact>,
        output_templates: Vec<String>,
        command: String,
        label: Label,
        execution_info: IndexMap<String, String>,
        requires_action_output: bool,
    ) -> ExtraActionSpec {
        ExtraActionSpec(Arc::new(ExtraActionSpecData {
            resolved_tools,
            resolved_data,
            output_templates,
            command,
            label,
            execution_info,
            requires_action_output,
        }))
    }

    /// Java `==` (identity).
    pub fn same(&self, other: &ExtraActionSpec) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    // Java: com/google/devtools/build/lib/analysis/extra/ExtraActionSpec.java:73 ExtraActionSpec.getLabel
    pub fn get_label(&self) -> &Label {
        &self.0.label
    }

    /// Adds an extra_action to the action graph based on the action to shadow.
    // Java: com/google/devtools/build/lib/analysis/extra/ExtraActionSpec.java:78 ExtraActionSpec.addExtraAction
    pub fn add_extra_action(
        &self,
        owning_rule: &RuleContext,
        action_to_shadow: &ActionRef,
    ) -> Result<Vec<Artifact>, InterruptedException> {
        // new LinkedHashSet<>()
        let mut extra_action_outputs: IndexSet<Artifact> = IndexSet::new();
        let mut proto_outputs: Vec<Artifact> = Vec::new();
        let mut extra_action_inputs: NestedSetBuilder<Artifact> = NestedSetBuilder::stable_order();

        let owner_label = owning_rule.get_label().clone();
        if self.0.requires_action_output {
            extra_action_inputs.add_all(action_to_shadow.get_outputs());
        }

        extra_action_inputs.add_transitive(&self.0.resolved_tools);
        extra_action_inputs.add_all(self.0.resolved_data.iter().cloned());

        let mut create_dummy_output = false;

        for output_template in &self.0.output_templates {
            // We create output for the extra_action based on the 'out_template' attribute.
            // See {link #getExtraActionOutputArtifact} for supported variables.
            extra_action_outputs.insert(self.get_extra_action_output_artifact(
                owning_rule,
                action_to_shadow,
                output_template,
            )?);
        }
        // extra_action has no output, we need to create some dummy output to keep the build
        // up-to-date.
        if extra_action_outputs.is_empty() {
            create_dummy_output = true;
            extra_action_outputs.insert(self.get_extra_action_output_artifact(
                owning_rule,
                action_to_shadow,
                "$(ACTION_ID).dummy",
            )?);
        }

        // We generate a file containing a protocol buffer describing the action that is being
        // shadowed. It is up to each action being shadowed to decide what contents to store here.
        let extra_action_info_file = self.get_extra_action_output_artifact(
            owning_rule,
            action_to_shadow,
            "$(ACTION_ID).xa",
        )?;
        owning_rule.register_action(Arc::new(ExtraActionInfoFileWriteAction::new(
            action_to_shadow.get_owner().clone(),
            extra_action_info_file.clone(),
            action_to_shadow.clone(),
            owning_rule.get_action_key_context().clone(),
        )));
        extra_action_inputs.add(extra_action_info_file.clone());
        proto_outputs.push(extra_action_info_file.clone());

        // Expand extra_action specific variables from the provided command-line.
        // See {@link #createExpandedCommand} for list of supported variables.
        let command =
            self.create_expanded_command(owning_rule, action_to_shadow, &extra_action_info_file)?;

        let command_helper = CommandHelper::builder(owning_rule).build();

        // Multiple actions in the same configured target need to have different names for the
        // artifact that might be created here, so we append something that should be unique for
        // each action.
        let action_uniquifier = format!(
            "{}.{}",
            action_to_shadow
                .get_primary_output()
                .get_exec_path()
                .get_base_name(),
            action_to_shadow.get_key(
                owning_rule.get_action_key_context(),
                /* input_metadata_provider= */ None,
            )?
        );

        let execution_platform = owning_rule.get_execution_platform();
        let sh_executable = sh_toolchain::get_path_for_platform(
            owning_rule.get_configuration(),
            execution_platform.as_ref(),
        );
        let constructor = build_bash_command_constructor(
            &self.0.execution_info,
            &sh_executable,
            &format!(".{action_uniquifier}.extra_action_script.sh"),
        );
        let argv = command_helper.build_command_line(
            &command,
            &mut extra_action_inputs,
            &constructor,
            get_os_from_constraints_or_host(execution_platform.as_deref()),
        );

        let command_message = format!("Executing extra_action {} on {}", self.0.label, owner_label);
        let configuration = owning_rule.get_configuration();
        let spawn_action_configuration = owning_rule.get_spawn_action_configuration();
        let outputs: Vec<Artifact> = extra_action_outputs.iter().cloned().collect();
        owning_rule.register_action(Arc::new(ExtraAction::new(
            owning_rule.get_action_owner(),
            extra_action_inputs.build(),
            outputs,
            action_to_shadow.clone(),
            create_dummy_output,
            blaze_actions::actions::command_line::of(argv),
            Arc::new(to_actions_env(configuration.get_action_environment())),
            spawn_action_configuration
                .modified_execution_info(&self.0.execution_info, self.0.label.get_name()),
            command_message,
            self.0.label.get_name().to_owned(),
        )));

        // ImmutableSet.builder().addAll(extraActionOutputs).addAll(protoOutputs).build()
        let mut result: IndexSet<Artifact> = extra_action_outputs;
        result.extend(proto_outputs);
        Ok(result.into_iter().collect())
    }

    /// Expand extra_action specific variables: $(EXTRA_ACTION_FILE): expands to a path of the
    /// file containing a protocol buffer describing the action being shadowed.
    /// $(output <out_template>): expands the output template to the execPath of the file. e.g.
    /// $(output $(ACTION_ID).out) ->
    /// <build_path>/extra_actions/bar/baz/devtools/build/test_A41234.out
    // Java: com/google/devtools/build/lib/analysis/extra/ExtraActionSpec.java:170 ExtraActionSpec.createExpandedCommand
    fn create_expanded_command(
        &self,
        owning_rule: &RuleContext,
        action: &ActionRef,
        extra_action_info_file: &Artifact,
    ) -> Result<String, InterruptedException> {
        let mut real_command = self.0.command.replace(
            "$(EXTRA_ACTION_FILE)",
            extra_action_info_file.get_exec_path_string(),
        );

        for output_template in &self.0.output_templates {
            let out_file = self
                .get_extra_action_output_artifact(owning_rule, action, output_template)?
                .get_exec_path_string()
                .to_owned();
            real_command = real_command.replace(&format!("$(output {output_template})"), &out_file);
        }
        Ok(real_command)
    }

    /// Creates an output artifact for the extra_action based on the output_template. The path
    /// will be in the following form: <output
    /// dir>/<target-configuration-specific-path>/extra_actions/<extra_action_label>/ +
    /// <configured_target_label>/<expanded_template>
    ///
    /// The template can use the following variables: $(ACTION_ID): a unique id for the
    /// extra_action.
    ///
    /// Sample: extra_action: foo/bar:extra template: $(ACTION_ID).analysis target: foo/bar:main
    /// expands to: output/configuration/extra_actions/\
    /// foo/bar/extra/foo/bar/4683026f7ac1dd1a873ccc8c3d764132.analysis
    // Java: com/google/devtools/build/lib/analysis/extra/ExtraActionSpec.java:197 ExtraActionSpec.getExtraActionOutputArtifact
    fn get_extra_action_output_artifact(
        &self,
        rule_context: &RuleContext,
        action: &ActionRef,
        template: &str,
    ) -> Result<Artifact, InterruptedException> {
        let action_id = get_action_id(
            rule_context.get_action_key_context(),
            &rule_context.get_action_owner(),
            action,
        )?;

        let template = template.replace("$(ACTION_ID)", &action_id);
        let template = template.replace("$(OWNER_LABEL_DIGEST)", &get_owner_digest(rule_context));

        Ok(self.get_root_relative_path(&template, rule_context))
    }

    // Java: com/google/devtools/build/lib/analysis/extra/ExtraActionSpec.java:208 ExtraActionSpec.getRootRelativePath
    fn get_root_relative_path(&self, template: &str, rule_context: &RuleContext) -> Artifact {
        let extra_action_package_fragment = self.0.label.get_package_identifier().get_source_root();
        let extra_action_prefix = extra_action_package_fragment
            .get_relative(&PathFragment::create(self.0.label.get_name()));
        let root_relative_path = PathFragment::create("extra_actions")
            .get_relative(&extra_action_prefix)
            .get_relative(&rule_context.get_package_directory())
            .get_relative(&PathFragment::create(template));
        // We need to use getDerivedArtifact here because extra actions are at
        // <EXTRA ACTION LABEL> / <RULE LABEL> instead of <RULE LABEL> / <EXTRA ACTION LABEL>.
        // Bummer.
        rule_context
            .get_analysis_environment()
            .get_derived_artifact(
                &root_relative_path,
                &crate::rule::artifact_roots::to_actions_root(
                    &rule_context
                        .get_configuration()
                        .get_output_directory(rule_context.get_rule().get_repository()),
                ),
            )
    }
}

impl TransitiveInfoProvider for ExtraActionSpec {
    fn java_class_name(&self) -> &'static str {
        "ExtraActionSpec"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Calculates a digest representing the rule context. We use the digest instead of the original
/// value as the original value might lead to a filename that is too long. By using a digest,
/// tools can deterministically find all extra_action outputs for a given target, without having
/// to open every file in the package.
// Java: com/google/devtools/build/lib/analysis/extra/ExtraActionSpec.java:233 ExtraActionSpec.getOwnerDigest
fn get_owner_digest(rule_context: &RuleContext) -> String {
    let mut f = Fingerprint::new();
    f.add_string(&rule_context.get_label().to_string());
    f.hex_digest_and_reset()
}

/// Creates a unique id for the action shadowed by this extra_action.
///
/// We need to have a unique id for the extra_action to use. We build this from the owner's label
/// and the shadowed action id (which is only guaranteed to be unique per target). Together with
/// the subfolder matching the original target's package name, we believe this is enough of a
/// uniqueness guarantee.
// Java: com/google/devtools/build/lib/analysis/extra/ExtraActionSpec.java:248 ExtraActionSpec.getActionId
pub fn get_action_id(
    action_key_context: &ActionKeyContext,
    owner: &ActionOwner,
    action: &ActionRef,
) -> Result<String, InterruptedException> {
    let mut f = Fingerprint::new();
    f.add_string(
        &owner
            .get_label()
            .map(|l| l.to_string())
            .unwrap_or_else(|| panic!("NullPointerException: owner label")),
    );
    let aspect_descriptors = owner.get_aspect_descriptors();
    f.add_int(aspect_descriptors.len() as i32);
    for aspect_descriptor in aspect_descriptors {
        f.add_string(&aspect_descriptor.description);
    }
    f.add_string(&action.get_key(action_key_context, /* input_metadata_provider= */ None)?);
    Ok(f.hex_digest_and_reset())
}
