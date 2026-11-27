//! Ports of `rules/genrule/GenRuleBase.java`, `rules/genrule/GenRuleAction.java` and
//! `bazel/rules/genrule/BazelGenRule.java`.

use std::sync::Arc;

use blaze_actions::actions::command_lines::CommandLines;
use blaze_actions::actions::path_mapper::OutputPathsMode;
use blaze_actions::analysis::actions::spawn_action::{ResourceSet, SpawnAction};
use blaze_actions::analysis::runfiles::{Runfiles, RunfilesBuilder};
use blaze_actions::{ActionEnvironment, ActionOwner, Artifact};
use blaze_analysis::configured_rule_class_provider::{
    CreateConfiguredTargetError, RuleConfiguredTargetFactory,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::platform::constraint_constants::get_os_from_constraints_or_host;
use blaze_analysis::providers::alias_provider::AliasProvider;
use blaze_analysis::providers::file_provider::{FileProvider, RunfilesProvider};
use blaze_analysis::rule::action_environments::to_actions_env;
use blaze_analysis::rule::command_helper::{CommandHelper, build_bash_command_constructor};
use blaze_analysis::rule::config_util;
use blaze_analysis::rule::configuration_make_variable_context::ConfigurationMakeVariableContext;
use blaze_analysis::rule::rule_configured_target_builder::{
    RuleConfiguredTargetBuildError, RuleConfiguredTargetBuilder,
};
use blaze_analysis::rule::rule_context::RuleContext;
use blaze_analysis::rule::rule_error_consumer::{RuleErrorConsumer, RuleErrorException};
use blaze_analysis::rule::sh_toolchain;
use blaze_analysis::rule::stringtemplate::{ExpansionException, TemplateContext};
use blaze_analysis::test::instrumented_files_info::{
    InstrumentationSpec, InstrumentedFilesCollector,
};
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_packages::attribute::AttributeMap;
use blaze_packages::types::{FileTypeSet, TriState, Type};
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use crate::generic::target_utils;

/// A spawn action for genrules. Genrules are handled specially in that inputs and outputs are
/// checked for directories.
///
/// PORT: Java's `GenRuleAction` is a `SpawnAction` subclass whose only override is
/// `getCommandLineLimits()` (`CommandLineLimits.UNLIMITED`). The action is created as a plain
/// `SpawnAction`: its command line is `CommandLines.of(argv)`, which has no `ParamFileInfo`, and
/// `CommandLines.expand` reads the limits only for command lines that have one
/// (actions/CommandLines.java:107), so the override cannot change a result.
// Java: com/google/devtools/build/lib/rules/genrule/GenRuleAction.java:32 GenRuleAction
pub struct GenRuleAction;

impl GenRuleAction {
    // Java: com/google/devtools/build/lib/rules/genrule/GenRuleAction.java:34 GenRuleAction.MNEMONIC
    pub const MNEMONIC: &'static str = "Genrule";

    // Java: com/google/devtools/build/lib/rules/genrule/GenRuleAction.java:36 GenRuleAction.<init>
    // PORT: GenRuleAction is a SpawnAction with its own mnemonic; the constructor returns that.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        owner: ActionOwner,
        tools: NestedSet<Artifact>,
        inputs: NestedSet<Artifact>,
        outputs: impl IntoIterator<Item = Artifact>,
        command_lines: CommandLines,
        env: Arc<ActionEnvironment>,
        execution_info: &IndexMap<String, String>,
        progress_message: String,
    ) -> SpawnAction {
        SpawnAction::new(
            owner,
            tools,
            inputs,
            outputs,
            // Java: AbstractAction.DEFAULT_RESOURCE_SET
            ResourceSet::default_resource_set(),
            command_lines,
            env,
            execution_info,
            Some(progress_message),
            Self::MNEMONIC,
            OutputPathsMode::OFF,
        )
        .with_java_class("com.google.devtools.build.lib.rules.genrule.GenRuleAction")
    }
}

// Java: com/google/devtools/build/lib/rules/genrule/GenRuleBase.java:256 GenRuleBase.CommandType
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommandType {
    Bash,
    WindowsBatch,
    WindowsPowershell,
}

/// Java `attributes().get(name, Type.BOOLEAN)`.
fn get_boolean(rule_context: &RuleContext, name: &str) -> bool {
    rule_context
        .attributes()
        .get(name, Type::Boolean)
        .and_then(|v| v.as_bool())
        .unwrap_or_else(|| panic!("IllegalArgumentException: no boolean attribute '{name}'"))
}

/// Java `attributes().get(name, Type.STRING)`.
fn get_string(rule_context: &RuleContext, name: &str) -> String {
    rule_context
        .attributes()
        .get(name, Type::String)
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| panic!("IllegalArgumentException: no string attribute '{name}'"))
}

/// A base implementation of genrule, to be used by specific implementing rules which can change
/// the semantics of `collect_sources`.
// Java: com/google/devtools/build/lib/rules/genrule/GenRuleBase.java:66 GenRuleBase
pub trait GenRuleBase: Send + Sync {
    /// Collects sources from src attribute.
    // Java: com/google/devtools/build/lib/rules/genrule/GenRuleBase.java:244 GenRuleBase.collectSources
    fn collect_sources(
        &self,
        srcs: &[ConfiguredTarget],
    ) -> Result<IndexMap<Label, NestedSet<Artifact>>, RuleErrorException>;

    /// Java's `final` `GenRuleBase.create`.
    // Java: com/google/devtools/build/lib/rules/genrule/GenRuleBase.java:70 GenRuleBase.create
    fn create_gen_rule(
        &self,
        rule_context: &RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        let files_to_build: NestedSet<Artifact> =
            NestedSetBuilder::wrap(Order::STABLE_ORDER, rule_context.get_output_artifacts());

        if files_to_build.is_empty() {
            rule_context.attribute_error("outs", "Genrules without outputs don't make sense");
        }
        if get_boolean(rule_context, "executable")
            && !files_to_build.is_empty()
            && !files_to_build.is_singleton()
        {
            rule_context.attribute_error(
                "executable",
                "if genrules produce executables, they are allowed only one output. \
                 If you need the executable=1 argument, then you should split this genrule into \
                 genrules producing single outputs",
            );
        }

        let cmd_type_and_attr = determine_command_type_and_attribute(rule_context);

        let label_map: IndexMap<Label, NestedSet<Artifact>> =
            self.collect_sources(&rule_context.get_prerequisites("srcs"))?;
        let mut resolved_srcs_builder: NestedSetBuilder<Artifact> =
            NestedSetBuilder::stable_order();
        for value in label_map.values() {
            resolved_srcs_builder.add_transitive(value);
        }
        let resolved_srcs: NestedSet<Artifact> = resolved_srcs_builder.build();

        let toolchain_prerequisites: Vec<ConfiguredTarget> = rule_context
            .get_toolchain_context()
            .expect("NullPointerException: ruleContext.getToolchainContext()")
            .prerequisite_targets()
            .iter()
            .map(|p| p.get_configured_target().clone())
            .collect();
        // The CommandHelper class makes an explicit copy of this in the constructor, so
        // flattening here should be benign.
        let command_helper: CommandHelper = CommandHelper::builder(rule_context)
            .add_tool_dependencies_attribute("tools")
            .add_tool_dependencies_attribute("toolchains")
            .add_tool_dependencies(toolchain_prerequisites)
            .add_label_map(
                label_map
                    .iter()
                    .map(|(k, v)| (k.clone(), v.to_list().to_vec()))
                    .collect(),
            )
            .build();

        if rule_context.has_errors() {
            return Ok(None);
        }

        // Java: `cmdTypeAndAttr` is null only after `attributeError`, i.e. when `hasErrors()`.
        let (cmd_type, cmd_attr) = cmd_type_and_attr.expect("NullPointerException: cmdTypeAndAttr");
        let expand_to_windows_path = cmd_type == CommandType::WindowsBatch;

        let base_command = get_string(rule_context, cmd_attr);

        // Expand template variables and functions.
        let command_resolver_context = CommandResolverContext::new(
            rule_context,
            resolved_srcs.clone(),
            files_to_build.clone(),
            expand_to_windows_path,
        );
        let mut command: String = rule_context
            .get_expander_for(Arc::new(command_resolver_context))
            .with_exec_locations_no_srcs(
                command_helper.get_label_map().clone(),
                expand_to_windows_path,
            )
            .expand(Some(cmd_attr), &base_command);

        // Heuristically expand things that look like labels.
        if get_boolean(rule_context, "heuristic_label_expansion") {
            command = command_helper.expand_labels_heuristically(&command);
        }

        if cmd_type == CommandType::Bash {
            // Add the genrule environment setup script before the actual shell command.
            command = format!(
                "source {}; {}",
                rule_context
                    .get_prerequisite_artifact("$genrule_setup")
                    .expect("NullPointerException: getPrerequisiteArtifact(\"$genrule_setup\")")
                    .get_exec_path(),
                command
            );
        }

        let message_attr = get_string(rule_context, "message");
        let message = if message_attr.is_empty() {
            "Executing genrule".to_owned()
        } else {
            message_attr
        };
        let label = rule_context.get_label();
        // PORT: Java's `OnDemandString` builds this string when the message is first needed; it
        // is a pure function of two immutable values, so it is built here.
        let progress_message = format!("{message} {label}");

        // PORT: java-hash-order. `Maps.newLinkedHashMap()` filled from
        // `TargetUtils.getExecutionInfo(rule)` (a copy of a `HashMap`).
        let mut execution_info: IndexMap<String, String> = IndexMap::new();
        for (k, v) in target_utils::get_execution_info(rule_context.get_rule()) {
            execution_info.insert(k, v);
        }

        if get_boolean(rule_context, "local") {
            execution_info.insert("local".to_owned(), String::new());
        }

        config_util::modify_execution_info(
            rule_context.get_configuration(),
            &mut execution_info,
            GenRuleAction::MNEMONIC,
        );

        let mut inputs: NestedSetBuilder<Artifact> = NestedSetBuilder::stable_order();
        inputs.add_transitive(&resolved_srcs);
        inputs.add_transitive(command_helper.get_resolved_tools());
        if cmd_type == CommandType::Bash {
            let genrule_setup_dep = rule_context
                .get_prerequisite("$genrule_setup")
                .expect("NullPointerException: getPrerequisite(\"$genrule_setup\")");
            let genrule_setup =
                blaze_analysis::configuredtargets::get_provider::<FileProvider>(&genrule_setup_dep)
                    .expect("NullPointerException: FileProvider of $genrule_setup");
            inputs.add_transitive(genrule_setup.get_files_to_build());
        }
        if rule_context.has_errors() {
            return Ok(None);
        }

        let constructor = match cmd_type {
            CommandType::WindowsBatch => {
                // PORT: skipped, Windows only.
                blaze_util::unported!("CommandHelper.buildWindowsBatchCommandConstructor")
            }
            CommandType::WindowsPowershell => {
                // PORT: skipped, Windows only.
                blaze_util::unported!("CommandHelper.buildWindowsPowershellCommandConstructor")
            }
            CommandType::Bash => {
                let sh_executable: PathFragment = sh_toolchain::get_path_for_platform(
                    rule_context.get_configuration(),
                    rule_context.get_execution_platform().as_ref(),
                );
                build_bash_command_constructor(
                    &execution_info,
                    &sh_executable,
                    ".genrule_script.sh",
                )
            }
        };
        let argv: Vec<String> = command_helper.build_command_line(
            &command,
            &mut inputs,
            &constructor,
            get_os_from_constraints_or_host(rule_context.get_execution_platform().as_deref()),
        );

        if is_stamping_enabled(rule_context) {
            inputs.add(
                rule_context
                    .get_analysis_environment()
                    .get_stable_workspace_status_artifact()
                    .expect("MissingDepException: workspace status"),
            );
            inputs.add(
                rule_context
                    .get_analysis_environment()
                    .get_volatile_workspace_status_artifact()
                    .expect("MissingDepException: workspace status"),
            );
        }

        rule_context.register_action(Arc::new(GenRuleAction::new(
            rule_context.get_action_owner(),
            command_helper.get_resolved_tools().clone(),
            inputs.build(),
            files_to_build.to_set(),
            CommandLines::of_args(argv),
            Arc::new(to_actions_env(
                rule_context.get_configuration().get_action_environment(),
            )),
            &execution_info,
            progress_message,
        )));

        let mut data_runfiles_builder = RunfilesBuilder::new(rule_context.get_workspace_name());
        data_runfiles_builder.add_transitive_artifacts(&files_to_build);
        let runfiles_provider = RunfilesProvider::with_data(
            // No runfiles provided if not a data dependency.
            Runfiles::empty(),
            // We only need to consider the outputs of a genrule. No need to visit the
            // dependencies of a genrule. They cross from the target into the exec configuration,
            // because the dependencies of a genrule are always built for the exec configuration.
            data_runfiles_builder.build(),
        );

        let instrumented_files_info = InstrumentedFilesCollector::collect(
            rule_context,
            &InstrumentationSpec::new(FileTypeSet::AnyFile).with_source_attributes(&["srcs"]),
            /* reported_to_actual_sources= */
            NestedSetBuilder::create(Order::STABLE_ORDER, std::iter::empty()),
        );

        let mut builder = RuleConfiguredTargetBuilder::new(rule_context);
        builder
            .set_files_to_build(files_to_build.clone())
            .set_runfiles_support(None, get_executable(rule_context, &files_to_build))
            .add_provider(runfiles_provider)
            .add_native_declared_provider(instrumented_files_info);

        match builder.build() {
            Ok(t) => Ok(t),
            Err(RuleConfiguredTargetBuildError::ActionConflict(m)) => {
                Err(CreateConfiguredTargetError::ActionConflict(m))
            }
            Err(RuleConfiguredTargetBuildError::Interrupted) => {
                Err(CreateConfiguredTargetError::Interrupted)
            }
        }
    }
}

// Java: com/google/devtools/build/lib/rules/genrule/GenRuleBase.java:247 GenRuleBase.isStampingEnabled
fn is_stamping_enabled(rule_context: &RuleContext) -> bool {
    // This intentionally does not call AnalysisUtils.isStampingEnabled(). That method returns
    // false in the exec configuration (regardless of the attribute value), which is the behavior
    // for binaries, but not genrules.
    let stamp: TriState = rule_context
        .attributes()
        .get("stamp", Type::Tristate)
        .and_then(|v| v.as_tri_state())
        .expect("IllegalArgumentException: no tristate attribute 'stamp'");
    stamp == TriState::Yes
        || (stamp == TriState::Auto && rule_context.get_configuration().stamp_binaries())
}

// Java: com/google/devtools/build/lib/rules/genrule/GenRuleBase.java:263 GenRuleBase.determineCommandTypeAndAttribute
fn determine_command_type_and_attribute(
    rule_context: &RuleContext,
) -> Option<(CommandType, &'static str)> {
    let attribute_map = rule_context.attributes();
    if rule_context.is_default_exec_group_executing_on_windows() {
        if attribute_map.is_attribute_value_explicitly_specified("cmd_ps") {
            return Some((CommandType::WindowsPowershell, "cmd_ps"));
        }
        if attribute_map.is_attribute_value_explicitly_specified("cmd_bat") {
            return Some((CommandType::WindowsBatch, "cmd_bat"));
        }
    }
    if attribute_map.is_attribute_value_explicitly_specified("cmd_bash") {
        return Some((CommandType::Bash, "cmd_bash"));
    }
    if attribute_map.is_attribute_value_explicitly_specified("cmd") {
        return Some((CommandType::Bash, "cmd"));
    }
    rule_context.attribute_error(
        "cmd",
        "missing value for `cmd` attribute, you can also set `cmd_ps` or `cmd_bat` on \
         Windows and `cmd_bash` on other platforms.",
    );
    None
}

/// Returns the executable artifact, if the rule is marked as executable and there is only one
/// artifact.
// Java: com/google/devtools/build/lib/rules/genrule/GenRuleBase.java:292 GenRuleBase.getExecutable
fn get_executable(
    rule_context: &RuleContext,
    files_to_build: &NestedSet<Artifact>,
) -> Option<Artifact> {
    if !get_boolean(rule_context, "executable") {
        return None;
    }
    if files_to_build.is_singleton() {
        Some(files_to_build.get_singleton().clone())
    } else {
        None
    }
}

/// Implementation of `ConfigurationMakeVariableContext` used to expand variables in a genrule
/// command string.
///
/// PORT: Java keeps the `RuleContext` and computes the rule directory on each lookup; it is a
/// pure function of the rule context, so the label and the directory are stored instead (a
/// template context must not borrow the rule context: `RuleContext.getExpander` keeps it).
// Java: com/google/devtools/build/lib/rules/genrule/GenRuleBase.java:303 GenRuleBase.CommandResolverContext
struct CommandResolverContext {
    /// The `ConfigurationMakeVariableContext` this class extends (`super`).
    super_context: Box<dyn TemplateContext + Send + Sync>,
    /// `ruleContext.getLabel()`.
    label: Label,
    /// `ruleContext.getBinOrGenfilesDirectory().getExecPath()
    /// .getRelative(ruleContext.getPackageDirectory())`.
    rule_dir_exec_path: PathFragment,
    resolved_srcs: NestedSet<Artifact>,
    files_to_build: NestedSet<Artifact>,
    windows_path: bool,
}

impl CommandResolverContext {
    // Java: com/google/devtools/build/lib/rules/genrule/GenRuleBase.java:310 CommandResolverContext.<init>
    fn new(
        rule_context: &RuleContext,
        resolved_srcs: NestedSet<Artifact>,
        files_to_build: NestedSet<Artifact>,
        windows_path: bool,
    ) -> CommandResolverContext {
        let rule_dir_package_path: PathFragment = rule_context.get_package_directory();
        let rule_dir_exec_path: PathFragment = rule_context
            .get_bin_or_genfiles_directory()
            .get_exec_path()
            .get_relative(&rule_dir_package_path);
        CommandResolverContext {
            super_context: Box::new(ConfigurationMakeVariableContext::with_extra_suppliers(
                rule_context.get_rule().get_package_declarations(),
                rule_context.get_configuration(),
                rule_context.get_default_template_variable_providers(),
                /* make_variable_suppliers= */ Vec::new(),
            )),
            label: rule_context.get_label().clone(),
            rule_dir_exec_path,
            resolved_srcs,
            files_to_build,
            windows_path,
        }
    }

    // Java: com/google/devtools/build/lib/rules/genrule/GenRuleBase.java:336 CommandResolverContext.lookupVariableImpl
    fn lookup_variable_impl(&self, variable_name: &str) -> Result<String, ExpansionException> {
        if variable_name == "SRCS" {
            return Ok(Artifact::join_exec_paths(
                " ",
                self.resolved_srcs.to_list().iter(),
            ));
        }

        if variable_name == "<" {
            return Self::expand_singleton_artifact(&self.resolved_srcs, "$<", "input file");
        }

        if variable_name == "OUTS" {
            return Ok(Artifact::join_exec_paths(
                " ",
                self.files_to_build.to_list().iter(),
            ));
        }

        if variable_name == "@" {
            return Self::expand_singleton_artifact(&self.files_to_build, "$@", "output file");
        }

        if variable_name == "RULEDIR" {
            // The output root directory. This variable expands to the package's root directory
            // in the genfiles tree.
            return Ok(self.rule_dir_exec_path.get_path_string().to_owned());
        }

        if variable_name == "@D" {
            // The output directory. If there is only one filename in outs,
            // this expands to the directory containing that file. If there are
            // multiple filenames, this variable instead expands to the
            // package's root directory in the genfiles tree, even if all the
            // generated files belong to the same subdirectory!
            if self.files_to_build.is_singleton() {
                let output_file = self.files_to_build.get_singleton();
                let relative_output_file = output_file.get_exec_path();
                if !relative_output_file.is_multi_segment() {
                    // This should never happen, since the path should contain at
                    // least a package name and a file name.
                    panic!(
                        "IllegalStateException: $(@D) for genrule {} has less than one segment",
                        self.label
                    );
                }
                return Ok(relative_output_file
                    .get_parent_directory()
                    .expect("multi-segment path has a parent directory")
                    .get_path_string()
                    .to_owned());
            } else {
                return Ok(self.rule_dir_exec_path.get_path_string().to_owned());
            }
        }

        self.super_context.lookup_variable(variable_name)
    }

    /// Returns the path of the sole element "artifacts", generating an exception with an
    /// informative error message iff the set is not a singleton. Used to expand "$<", "$@".
    // Java: com/google/devtools/build/lib/rules/genrule/GenRuleBase.java:391 CommandResolverContext.expandSingletonArtifact
    fn expand_singleton_artifact(
        artifacts: &NestedSet<Artifact>,
        variable: &str,
        artifact_name: &str,
    ) -> Result<String, ExpansionException> {
        if artifacts.is_empty() {
            return Err(ExpansionException(format!(
                "variable '{variable}' : no {artifact_name}"
            )));
        } else if !artifacts.is_singleton() {
            return Err(ExpansionException(format!(
                "variable '{variable}' : more than one {artifact_name}"
            )));
        }
        Ok(artifacts.get_singleton().get_exec_path_string().to_owned())
    }
}

impl TemplateContext for CommandResolverContext {
    // Java: com/google/devtools/build/lib/rules/genrule/GenRuleBase.java:328 CommandResolverContext.lookupVariable
    fn lookup_variable(&self, variable_name: &str) -> Result<String, ExpansionException> {
        let val = self.lookup_variable_impl(variable_name)?;
        if self.windows_path {
            return Ok(val.replace('/', "\\"));
        }
        Ok(val)
    }

    // Java: com/google/devtools/build/lib/analysis/ConfigurationMakeVariableContext.java:83 ConfigurationMakeVariableContext.lookupFunction
    fn lookup_function(&self, name: &str, param: &str) -> Result<String, ExpansionException> {
        self.super_context.lookup_function(name, param)
    }
}

/// An implementation of genrule for Bazel.
// Java: com/google/devtools/build/lib/bazel/rules/genrule/BazelGenRule.java:27 BazelGenRule
pub struct BazelGenRule;

impl GenRuleBase for BazelGenRule {
    // Java: com/google/devtools/build/lib/bazel/rules/genrule/BazelGenRule.java:30 BazelGenRule.collectSources
    fn collect_sources(
        &self,
        srcs: &[ConfiguredTarget],
    ) -> Result<IndexMap<Label, NestedSet<Artifact>>, RuleErrorException> {
        let mut label_map: IndexMap<Label, NestedSet<Artifact>> =
            IndexMap::with_capacity(srcs.len());

        for dep in srcs {
            let files: NestedSet<Artifact> =
                blaze_analysis::configuredtargets::get_provider::<FileProvider>(dep)
                    .expect("NullPointerException: FileProvider")
                    .get_files_to_build()
                    .clone();
            let key = AliasProvider::get_dependency_label(dep);
            // Java: ImmutableMap.Builder.buildOrThrow.
            if label_map.contains_key(&key) {
                panic!("IllegalArgumentException: Multiple entries with same key: {key}");
            }
            label_map.insert(key, files);
        }

        Ok(label_map)
    }
}

impl RuleConfiguredTargetFactory for BazelGenRule {
    // Java: com/google/devtools/build/lib/rules/genrule/GenRuleBase.java:70 GenRuleBase.create
    fn create(
        &self,
        rule_context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        let rule_context: &RuleContext = rule_context;
        self.create_gen_rule(rule_context)
    }
}

#[cfg(test)]
mod tests {
    // Unit-level checks of `CommandResolverContext` with the expectations of
    // src/test/java/com/google/devtools/build/lib/bazel/rules/genrule/
    // {GenRuleCommandSubstitutionTest, GenRuleConfiguredTargetTest}.java, followed by the Java
    // tests themselves (they analyse BUILD files: `BuildViewTestCase`). End to end, genrule is
    // compared with Bazel by scripts/w4/scenarios/genrule.
    use super::*;
    use blaze_actions::{ArtifactOwner, ArtifactRoot};
    use blaze_analysis::rule::stringtemplate::TemplateExpander;
    use blaze_vfs::{DigestHashFunction, Path, Root, UnixFileSystem};

    /// Stands in for the `ConfigurationMakeVariableContext` superclass: no variables.
    struct NoVariables;

    impl TemplateContext for NoVariables {
        fn lookup_variable(&self, name: &str) -> Result<String, ExpansionException> {
            // Java: ConfigurationMakeVariableContext.lookupVariable's failure.
            Err(ExpansionException(format!("$({name}) not defined")))
        }

        fn lookup_function(&self, name: &str, _param: &str) -> Result<String, ExpansionException> {
            Err(ExpansionException(format!("$({name}) not defined")))
        }
    }

    /// An artifact with the given exec path (only exec paths are read by the context).
    fn artifact(exec_path: &str) -> Artifact {
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let root = ArtifactRoot::as_source_root(Root::from_path(Path::create(
            PathFragment::create("/workspace"),
            &fs,
        )));
        Artifact::new_source_artifact(
            root,
            PathFragment::create(exec_path),
            ArtifactOwner::NullOwner,
        )
    }

    fn context(srcs: &[&str], outs: &[&str], windows_path: bool) -> CommandResolverContext {
        CommandResolverContext {
            super_context: Box::new(NoVariables),
            label: Label::parse_canonical("//test:test").unwrap(),
            rule_dir_exec_path: PathFragment::create("bazel-out/k8-fastbuild/bin/test"),
            resolved_srcs: NestedSetBuilder::wrap(
                Order::STABLE_ORDER,
                srcs.iter().map(|s| artifact(s)).collect::<Vec<_>>(),
            ),
            files_to_build: NestedSetBuilder::wrap(
                Order::STABLE_ORDER,
                outs.iter().map(|s| artifact(s)).collect::<Vec<_>>(),
            ),
            windows_path,
        }
    }

    fn expand(cmd: &str, ctx: &CommandResolverContext) -> Result<String, ExpansionException> {
        TemplateExpander::expand(cmd, ctx).map(|e| e.expansion)
    }

    const OUT: &str = "bazel-out/k8-fastbuild/bin/test/out";

    #[test]
    fn mnemonic() {
        assert_eq!(GenRuleAction::MNEMONIC, "Genrule");
    }

    /// GenRuleCommandSubstitutionTest.testOuts: `$(OUTS) # $@` is the output twice.
    #[test]
    fn command_resolver_context_outs() {
        let ctx = context(&[], &[OUT], false);
        assert_eq!(
            expand("$(OUTS) # $@", &ctx).unwrap(),
            format!("{OUT} # {OUT}")
        );
    }

    /// GenRuleCommandSubstitutionTest.testSrcs: `$(SRCS) # $<` is `test/src # test/src`.
    #[test]
    fn command_resolver_context_srcs() {
        let ctx = context(&["test/src"], &[OUT], false);
        assert_eq!(expand("$(SRCS) # $<", &ctx).unwrap(), "test/src # test/src");
    }

    /// GenRuleCommandSubstitutionTest.testDollarDollar.
    #[test]
    fn command_resolver_context_dollar_dollar() {
        let ctx = context(&[], &[OUT], false);
        assert_eq!(expand("$$DOLLAR", &ctx).unwrap(), "$DOLLAR");
    }

    /// GenRuleCommandSubstitutionTest.testDollarLessThanWithZeroInputs.
    #[test]
    fn command_resolver_context_dollar_less_than_with_zero_inputs() {
        let ctx = context(&[], &[OUT], false);
        assert_eq!(
            expand("$<", &ctx).unwrap_err().0,
            "variable '$<' : no input file"
        );
    }

    /// GenRuleCommandSubstitutionTest.testDollarLessThanWithMultipleInputs (and the oracle:
    /// scripts/w4/scenarios/genrule step 18).
    #[test]
    fn command_resolver_context_dollar_less_than_with_multiple_inputs() {
        let ctx = context(&["test/src1", "test/src2"], &[OUT], false);
        assert_eq!(
            expand("$<", &ctx).unwrap_err().0,
            "variable '$<' : more than one input file"
        );
    }

    /// GenRuleCommandSubstitutionTest.testDollarAtWithMultipleOutputs (and the oracle:
    /// scripts/w4/scenarios/genrule step 19).
    #[test]
    fn command_resolver_context_dollar_at_with_multiple_outputs() {
        let ctx = context(
            &[],
            &[
                "bazel-out/k8-fastbuild/bin/test/out.1",
                "bazel-out/k8-fastbuild/bin/test/out.2",
            ],
            false,
        );
        assert_eq!(
            expand("$@", &ctx).unwrap_err().0,
            "variable '$@' : more than one output file"
        );
    }

    /// GenRuleCommandSubstitutionTest.testShellVariables.
    #[test]
    fn command_resolver_context_shell_variables() {
        let ctx = context(&[], &[OUT], false);
        assert_eq!(
            expand("for file in a b c;do echo $$file;done", &ctx).unwrap(),
            "for file in a b c;do echo $file;done"
        );
        assert_eq!(expand("$${file%:.*8}", &ctx).unwrap(), "${file%:.*8}");
        assert_eq!(
            expand("$$(basename file)", &ctx).unwrap(),
            "$(basename file)"
        );
        assert_eq!(
            expand("$(basename file)", &ctx).unwrap_err().0,
            "$(basename) not defined"
        );
    }

    /// GenRuleConfiguredTargetTest.testOutputDirExpansion: `$(@D)` is the directory of a single
    /// output, and the package's directory (not the outputs' common parent) for several.
    #[test]
    fn command_resolver_context_output_dir_expansion() {
        let single = context(
            &[],
            &["bazel-out/k8-fastbuild/bin/test/bar/bar_out.txt"],
            false,
        );
        assert_eq!(
            expand("touch $(@D)", &single).unwrap(),
            "touch bazel-out/k8-fastbuild/bin/test/bar"
        );
        let multi = context(
            &[],
            &[
                "bazel-out/k8-fastbuild/bin/test/logs/baz_out.txt",
                "bazel-out/k8-fastbuild/bin/test/logs/baz.log",
            ],
            false,
        );
        assert_eq!(
            expand("touch $(@D)", &multi).unwrap(),
            "touch bazel-out/k8-fastbuild/bin/test"
        );
    }

    /// GenRuleConfiguredTargetTest.testRuleDirExpansion: `$(RULEDIR)` is the package's directory
    /// in the output tree whatever the outputs are.
    #[test]
    fn command_resolver_context_rule_dir_expansion() {
        let single = context(
            &[],
            &["bazel-out/k8-fastbuild/bin/test/bar/bar_out.txt"],
            false,
        );
        assert_eq!(
            expand("touch $(RULEDIR)", &single).unwrap(),
            "touch bazel-out/k8-fastbuild/bin/test"
        );
    }

    /// GenRuleBase.java:330: with a Windows batch command, variables use backslashes.
    #[test]
    fn command_resolver_context_windows_path() {
        let ctx = context(&["test/src"], &[OUT], true);
        assert_eq!(expand("$<", &ctx).unwrap(), "test\\src");
    }

    // The GenRule*Test JUnit tests are in crates/blaze_sky_top/src/tests/gen_rule_*_test.rs.
}
