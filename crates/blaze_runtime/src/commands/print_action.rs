//! `blaze print_action`: prints the extra-action information of the actions that compile the
//! given targets or files.
//!
//! Ports `runtime/commands/PrintActionCommand.java` and `analysis/PrintActionVisitor.java` (with
//! `actions/ActionGraphVisitor.java` and `actions/BipartiteVisitor.java`, which only it uses).

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use blaze_actions::actions::action::CommandLineError;
use blaze_actions::{ActionKeyContext, ActionRef, Artifact};
use blaze_analysis::aspect_key_creator::{AspectKey, TopLevelAspectsKey, TopLevelAspectsValue};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::providers::file_provider::FileProvider;
use blaze_analysis::providers::output_group_info::{FILES_TO_COMPILE, OutputGroupInfo};
use blaze_cmdline::Label;
use blaze_events::{Event, EventHandler as _};
use blaze_optdefs::command::print_action_command::PrintActionCommandPrintActionOptions;
use blaze_packages::starlark_defs::builtin_providers::OUTPUT_GROUP_INFO;
use blaze_packages::target::Target;
use blaze_protos::blaze::{DetailedExtraActionInfo, ExtraActionSummary};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_sky_exec::action_template_expansion_value::{
    ActionTemplateExpansionKey, ActionTemplateExpansionValue,
};
use blaze_sky_loading::package_manager::{NoSuchThingOrInterrupted, TargetProvider as _};
use blaze_sky_top::skyframe_executor::SkyframeExecutor;
use blaze_skyframe::SkyKey;

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::commands::build::{RunBuild, RunBuildOutcome, run_build};
use crate::exit_code::DetailedExitCode;
use crate::options_result::OptionsParsingResult;

/// Implements 'blaze print_action' by finding the Configured target[s] for the file[s] listed.
// Java: com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java:80 PrintActionCommand
pub struct PrintActionCommand;

impl BlazeCommand for PrintActionCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &crate::command_registry::PRINT_ACTION_COMMAND
    }

    // Java: com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java:102 PrintActionCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        let loading_options: blaze_optdefs::command::loading_options::LoadingOptions = options
            .get_options()
            .expect("NullPointerException: LoadingOptions");
        let print_action_options: PrintActionCommandPrintActionOptions = options
            .get_options()
            .expect("NullPointerException: PrintActionOptions");
        let keep_going = options
            .get_options::<blaze_optdefs::command::keep_going_option::KeepGoingOption>()
            .expect("NullPointerException: KeepGoingOption")
            .keep_going;
        let runner = PrintActionRunner {
            compile_one_dependency: loading_options.compile_one_dependency,
            requested_targets: options.get_residue(),
            keep_going,
            // Sets.newHashSet(printActionOptions.printActionMnemonics)
            print_action_mnemonics: print_action_options
                .print_action_mnemonics
                .iter()
                .cloned()
                .collect(),
        };
        BlazeCommandResult::detailed_exit_code(runner.print_actions_for_targets(env, options))
    }
}

/// Contains all the logic to get extra_action information for print actions. Maintains
/// required state to perform required analyses.
// Java: com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java:117 PrintActionCommand.PrintActionRunner
struct PrintActionRunner {
    compile_one_dependency: bool,
    requested_targets: Vec<String>,
    keep_going: bool,
    print_action_mnemonics: HashSet<String>,
}

/// `PrintActionException`: a message and a `FailureDetails.PrintActionCommand.Code`.
// Java: com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java:390 PrintActionCommand.PrintActionException
struct PrintActionException {
    message: String,
    detailed_code: failure_details::print_action_command::Code,
}

impl PrintActionException {
    // Java: com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java:398 PrintActionCommand.PrintActionException.createFailureDetail
    fn create_failure_detail(&self) -> FailureDetail {
        FailureDetail {
            message: self.message.clone(),
            category: Some(Category::PrintActionCommand(
                failure_details::PrintActionCommand {
                    code: self.detailed_code as i32,
                },
            )),
        }
    }
}

/// The exceptions of `gatherActionsForTargets` (and `CommandLineExpansionException` of the
/// methods it calls, which it turns into a `PrintActionException`).
enum GatherError {
    PrintAction(PrintActionException),
    /// `CommandLineExpansionException` (its message).
    CommandLineExpansion(String),
    Interrupted,
}

impl From<CommandLineError> for GatherError {
    fn from(e: CommandLineError) -> Self {
        match e {
            CommandLineError::CommandLineExpansion(e) => {
                GatherError::CommandLineExpansion(e.message)
            }
            CommandLineError::Interrupted(_) => GatherError::Interrupted,
        }
    }
}

impl PrintActionRunner {
    /// `actionMnemonicMatcher`.
    // Java: com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java:141 PrintActionCommand.PrintActionRunner.<init>
    fn action_mnemonic_matcher(&self, action: &ActionRef) -> bool {
        self.print_action_mnemonics.is_empty()
            || self.print_action_mnemonics.contains(&action.get_mnemonic())
    }

    // Java: com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java:151 PrintActionCommand.PrintActionRunner.printActionsForTargets
    fn print_actions_for_targets(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> DetailedExitCode {
        // gatherActionsForTargets runs BuildTool.processRequest; the rest of this method runs on
        // its BuildResult.
        self.gather_actions_for_targets(env, options)
    }

    /// `gatherActionsForTargets`, then the rest of `printActionsForTargets` on its result.
    ///
    /// PORT: Java builds the request and runs `new BuildTool(env).processRequest(request, null,
    /// options)`; Blaze runs the build command's driver (crate::commands::build::run_build) on
    /// the targets, with the hook that `run` uses, and continues on the `BuildResult` in
    /// `after_build`.
    // Java: com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java:179 PrintActionCommand.PrintActionRunner.gatherActionsForTargets
    fn gather_actions_for_targets(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> DetailedExitCode {
        let validator = NoTargetValidator;
        let targets = self.requested_targets.clone();
        let result = run_build(
            env,
            options,
            /* run_tests= */ false,
            Some(RunBuild {
                targets: targets.clone(),
                validator: &validator,
                repo_precomputed: Default::default(),
                after_build: Box::new(|outcome: RunBuildOutcome<'_>| {
                    // summaryBuilder = ExtraActionSummary.newBuilder()
                    let mut summary = ExtraActionSummary::default();
                    let gathered =
                        self.gather_actions_after_build(&outcome, &targets, &mut summary);
                    BlazeCommandResult::detailed_exit_code(self.print_actions(
                        &*outcome.env,
                        &outcome,
                        gathered,
                        &summary,
                    ))
                }),
            }),
        );
        result.get_detailed_exit_code().clone()
    }

    /// The rest of `printActionsForTargets` once `gatherActionsForTargets` returned or threw.
    // Java: com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java:151 PrintActionCommand.PrintActionRunner.printActionsForTargets
    fn print_actions(
        &self,
        env: &CommandEnvironment,
        outcome: &RunBuildOutcome<'_>,
        gathered: Result<(), GatherError>,
        summary: &ExtraActionSummary,
    ) -> DetailedExitCode {
        match gathered {
            Ok(()) => {}
            Err(GatherError::PrintAction(e)) => {
                return DetailedExitCode::of(e.create_failure_detail());
            }
            Err(GatherError::Interrupted) => {
                let message = "print_action: action gathering interrupted";
                env.get_reporter().handle(&Event::error(message));
                return interrupted_detailed_exit_code(message);
            }
            Err(GatherError::CommandLineExpansion(_)) => {
                unreachable!("turned into a PrintActionException by gatherActionsForTargets")
            }
        }
        if self.has_fatal_build_failure(outcome) {
            env.get_reporter()
                .handle(&Event::error("Build failed when printing actions"));
            return outcome.detailed_exit_code.clone();
        }
        let action = blaze_protos::text_format::printer().print_to_string(summary);
        if !action.is_empty() {
            env.get_reporter().get_out_err().print_out(&action);
            return outcome.detailed_exit_code.clone();
        }
        let message = "no actions to print were found";
        env.get_reporter().handle(&Event::error(message));
        DetailedExitCode::of(FailureDetail {
            message: message.to_string(),
            category: Some(Category::PrintActionCommand(
                failure_details::PrintActionCommand {
                    code: failure_details::print_action_command::Code::ActionsNotFound as i32,
                },
            )),
        })
    }

    /// The part of `gatherActionsForTargets` after `processRequest`.
    // Java: com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java:194 PrintActionCommand.PrintActionRunner.gatherActionsForTargets
    fn gather_actions_after_build(
        &self,
        outcome: &RunBuildOutcome<'_>,
        targets: &[String],
        summary: &mut ExtraActionSummary,
    ) -> Result<(), GatherError> {
        if self.has_fatal_build_failure(outcome) {
            return Ok(());
        }
        let env = &*outcome.env;
        // env.getSkyframeExecutor().getActionGraph(env.getReporter())
        let action_graph = SkyframeActionGraph {
            skyframe_executor: outcome.skyframe_executor,
        };

        let actual_targets = outcome
            .build_result
            .get_actual_targets()
            .expect("checked by hasFatalBuildFailure");
        for view in actual_targets {
            let configured_target = configured_target_of(&outcome.analyzed_targets, view);
            // filesToCompile: empty (NestedSetBuilder.emptySet) without an OutputGroupInfo.
            let files_to_compile_is_empty = output_group_info_of(&configured_target)
                .is_none_or(|info| info.get_output_group(FILES_TO_COMPILE).is_empty());
            if !files_to_compile_is_empty {
                let action_key_context = outcome
                    .skyframe_executor
                    .get_action_key_context()
                    .expect("NullPointerException: getActionKeyContext")
                    .clone();
                let gathered = if self.compile_one_dependency {
                    self.gather_actions_for_files(
                        &configured_target,
                        env,
                        &action_graph,
                        &action_key_context,
                        targets,
                        summary,
                    )
                } else {
                    let target = match outcome
                        .skyframe_executor
                        .get_package_manager()
                        .get_target(env.get_reporter().clone(), &configured_target.get_label())
                    {
                        Ok(target) => target,
                        Err(NoSuchThingOrInterrupted::Interrupted) => {
                            return Err(GatherError::Interrupted);
                        }
                        Err(NoSuchThingOrInterrupted::NoSuchThing(e)) => {
                            let message = format!("Failed to find target to gather actions: {}", e);
                            env.get_reporter().handle(&Event::error(message.clone()));
                            return Err(GatherError::PrintAction(PrintActionException {
                                message,
                                detailed_code:
                                    failure_details::print_action_command::Code::TargetNotFound,
                            }));
                        }
                    };
                    self.gather_actions_for_target(
                        &configured_target,
                        &target,
                        &action_graph,
                        &action_key_context,
                        summary,
                    )
                };
                match gathered {
                    Ok(()) => {}
                    Err(GatherError::CommandLineExpansion(e_message)) => {
                        // "Error expanding command line: " + e (Throwable.toString()).
                        let message = format!(
                            "Error expanding command line: \
                             com.google.devtools.build.lib.actions.CommandLineExpansionException: {}",
                            e_message
                        );
                        env.get_reporter().handle(&Event::error(message.clone()));
                        return Err(GatherError::PrintAction(PrintActionException {
                            message,
                            detailed_code:
                                failure_details::print_action_command::Code::CommandLineExpansionFailure,
                        }));
                    }
                    Err(e) => return Err(e),
                }
            } else {
                let message = format!(
                    "{} is not a supported target kind",
                    configured_target_to_string(&configured_target)
                );
                env.get_reporter().handle(&Event::error(message.clone()));
                return Err(GatherError::PrintAction(PrintActionException {
                    message,
                    detailed_code:
                        failure_details::print_action_command::Code::TargetKindUnsupported,
                }));
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java:257 PrintActionCommand.PrintActionRunner.gatherActionsForFiles
    fn gather_actions_for_files(
        &self,
        configured_target: &ConfiguredTarget,
        env: &CommandEnvironment,
        action_graph: &SkyframeActionGraph<'_>,
        action_key_context: &ActionKeyContext,
        files: &[String],
        summary: &mut ExtraActionSummary,
    ) -> Result<(), GatherError> {
        // new LinkedHashSet<>(files)
        let mut files_desired: indexmap::IndexSet<String> = files.iter().cloned().collect();
        let mut filter = ActionFilter {
            files_desired: &mut files_desired,
            runner: self,
        };
        self.gather_actions_for_file(
            configured_target,
            &mut filter,
            env,
            action_graph,
            action_key_context,
            summary,
        )
    }

    // Java: com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java:268 PrintActionCommand.PrintActionRunner.gatherActionsForTarget
    fn gather_actions_for_target(
        &self,
        configured_target: &ConfiguredTarget,
        target: &Target,
        action_graph: &SkyframeActionGraph<'_>,
        action_key_context: &ActionKeyContext,
        summary: &mut ExtraActionSummary,
    ) -> Result<(), GatherError> {
        if !matches!(target, Target::Rule(_)) {
            return Ok(());
        }

        let mut visitor = PrintActionVisitor::new(action_graph, configured_target, |a| {
            self.action_mnemonic_matcher(a)
        });

        // TODO(jvg): do we want to support ruleConfiguredTarget.getOutputArtifacts()?
        // We do for extra actions, but as we're past the action graph building phase,
        // we cannot call it without risking to trigger creation of OutputArtifacts post
        // graph building phase (not allowed). Right now we do not need them for our scenarios.
        let files_to_build = configured_target
            .get_provider::<FileProvider>()
            .expect("NullPointerException: FileProvider")
            .get_files_to_build()
            .to_list();
        visitor.visit_white_nodes(&files_to_build[..]);

        for action in visitor.get_actions() {
            // `action instanceof Action`: not an action template (an `ActionAnalysisMetadata`
            // that is not an `ActionExecutionMetadata`).
            if action.is_action_execution_metadata() {
                add_extra_action_info(summary, &action, action_key_context)?;
            }
        }
        Ok(())
    }

    /// Looks for files to compile in the given configured target and outputs the corresponding
    /// extra_action if the filter evaluates to `true`.
    // Java: com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java:306 PrintActionCommand.PrintActionRunner.gatherActionsForFile
    fn gather_actions_for_file(
        &self,
        configured_target: &ConfiguredTarget,
        filter: &mut ActionFilter<'_>,
        env: &CommandEnvironment,
        action_graph: &SkyframeActionGraph<'_>,
        action_key_context: &ActionKeyContext,
        summary: &mut ExtraActionSummary,
    ) -> Result<(), GatherError> {
        let artifacts = output_group_info_of(configured_target)
            .expect("NullPointerException: OutputGroupInfo")
            .get_output_group(FILES_TO_COMPILE);

        if artifacts.is_empty() {
            return Ok(());
        }

        for artifact in artifacts.to_list().iter() {
            let action = action_graph.get_generating_action(artifact);
            if filter.should_output(
                action.as_ref(),
                configured_target,
                env,
                action_graph.skyframe_executor,
            )? {
                // `action instanceof Action`
                if let Some(action) = &action
                    && action.is_action_execution_metadata()
                {
                    add_extra_action_info(summary, action, action_key_context)?;
                }
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java:331 PrintActionCommand.PrintActionRunner.hasFatalBuildFailure
    fn has_fatal_build_failure(&self, outcome: &RunBuildOutcome<'_>) -> bool {
        outcome.build_result.get_actual_targets().is_none()
            || (!outcome.build_result.get_success() && !self.keep_going)
    }
}

/// `DetailedExtraActionInfo.Builder detail = DetailedExtraActionInfo.newBuilder();
/// detail.setAction(((Action) action).getExtraActionInfo(actionKeyContext));
/// summaryBuilder.addAction(detail)`.
// Java: com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java:290 PrintActionCommand.PrintActionRunner.gatherActionsForTarget
fn add_extra_action_info(
    summary: &mut ExtraActionSummary,
    action: &ActionRef,
    action_key_context: &ActionKeyContext,
) -> Result<(), GatherError> {
    summary.action.push(DetailedExtraActionInfo {
        triggering_file: None,
        action: action.get_extra_action_info(action_key_context)?,
    });
    Ok(())
}

/// `BuildTool.processRequest(request, null, options)`: no target validator.
struct NoTargetValidator;

impl blaze_sky_top::build_tool::TargetValidator for NoTargetValidator {
    fn validate_targets(
        &self,
        _targets: &[Target],
        _keep_going: bool,
    ) -> Result<(), blaze_sky_top::project::LoadingFailedException> {
        Ok(())
    }
}

/// The `ConfiguredTarget` behind a view of the build result.
fn configured_target_of(
    analyzed_targets: &[ConfiguredTarget],
    view: &blaze_buildtool::buildtool::build_result::ConfiguredTargetRef,
) -> ConfiguredTarget {
    let key = view.get_lookup_key();
    analyzed_targets
        .iter()
        .find(|ct| *ct.unwrap_if_merged().get_lookup_key() == key)
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "IllegalStateException: actual target {} was not analyzed",
                view.get_label()
            )
        })
}

/// `OutputGroupInfo.get(configuredTarget)`.
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:132 OutputGroupInfo.get
fn output_group_info_of(target: &ConfiguredTarget) -> Option<OutputGroupInfo> {
    let info = target.get(&OUTPUT_GROUP_INFO.key())?;
    Some(
        blaze_analysis::info::downcast_info::<OutputGroupInfo>(&info)
            .expect("ClassCastException: OutputGroupInfo")
            .clone(),
    )
}

/// `ConfiguredTarget.toString()`.
fn configured_target_to_string(configured_target: &ConfiguredTarget) -> String {
    blaze_analysis::configuredtargets::configured_target_to_string(configured_target)
}

/// `env.getSkyframeExecutor().getActionGraph(eventHandler)`.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2737 SkyframeExecutor.getActionGraph
struct SkyframeActionGraph<'a> {
    skyframe_executor: &'a SkyframeExecutor,
}

impl SkyframeActionGraph<'_> {
    /// `SkyframeExecutor.getGeneratingAction`: the action at the artifact's generating action
    /// key in its owner's value.
    ///
    /// PORT: Java evaluates the owner's key (done after the analysis and the build, so the
    /// evaluation only reads it); the value is read from the graph, for the owners whose values
    /// the build creates: configured targets, aspects, top-level aspects, action template
    /// expansions and the workspace status. Other owners (the coverage report) fail loudly.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2702 SkyframeExecutor.getGeneratingAction
    fn get_generating_action(&self, artifact: &Artifact) -> Option<ActionRef> {
        if artifact.is_source_artifact() {
            return None;
        }
        let generating_action_key = artifact.get_generating_action_key();
        let lookup_key = generating_action_key.get_action_lookup_key();
        let graph = self.skyframe_executor.get_walkable_graph();
        // ActionLookupValue.getActions() of the key's value (result.hasError(): null).
        let any = lookup_key.as_any();
        let actions: Vec<ActionRef> = if let Some(key) = any.downcast_ref::<ConfiguredTargetKey>() {
            blaze_sky_top::skyframe_executor::get_configured_target_value(
                &*graph,
                &SkyKey::new(key.clone()),
            )?
            .get_actions()
            .to_vec()
        } else if let Some(key) = any.downcast_ref::<AspectKey>() {
            graph
                .get_value(&SkyKey::new(key.clone()))?
                .downcast::<blaze_analysis::aspect_value::AspectValue>()
                .unwrap_or_else(|_| panic!("ASPECT value of {key:?}"))
                .get_actions()
                .to_vec()
        } else if let Some(key) = any.downcast_ref::<TopLevelAspectsKey>() {
            graph
                .get_value(&SkyKey::new(key.clone()))?
                .downcast::<TopLevelAspectsValue>()
                .unwrap_or_else(|_| panic!("TOP_LEVEL_ASPECTS value of {key:?}"))
                .get_actions()
                .to_vec()
        } else if let Some(key) = any.downcast_ref::<ActionTemplateExpansionKey>() {
            use blaze_sky_exec::analysis_seam::ActionLookupValue as _;
            graph
                .get_value(&SkyKey::new(key.clone()))?
                .downcast::<ActionTemplateExpansionValue>()
                .unwrap_or_else(|_| panic!("ACTION_TEMPLATE_EXPANSION value of {key:?}"))
                .get_actions()
                .to_vec()
        } else if any.is::<blaze_analysis::analysis_services::WorkspaceStatusKey>() {
            graph
                .get_value(&SkyKey::new(
                    blaze_analysis::analysis_services::WorkspaceStatusKey,
                ))?
                .downcast::<blaze_analysis::analysis_services::WorkspaceStatusValue>()
                .unwrap_or_else(|_| panic!("BUILD_INFO value"))
                .actions
                .clone()
        } else {
            blaze_util::unported!(
                "SkyframeExecutor.getGeneratingAction",
                "generating action owned by {} ({})",
                lookup_key,
                lookup_key.function_name()
            );
        };
        Some(actions[generating_action_key.get_action_index() as usize].clone())
    }
}

/// A bipartite graph visitor which accumulates actions with matching mnemonics for a target.
///
/// PORT: `ActionGraphVisitor` and `BipartiteVisitor` (its only use) are folded in; the visited
/// sets are keyed by identity (actions) and by artifact equality, as Java's `HashMap`s are.
// Java: com/google/devtools/build/lib/analysis/PrintActionVisitor.java:28 PrintActionVisitor
// Java: com/google/devtools/build/lib/actions/ActionGraphVisitor.java:23 ActionGraphVisitor
// Java: com/google/devtools/build/lib/actions/BipartiteVisitor.java:28 BipartiteVisitor
struct PrintActionVisitor<'a, F: Fn(&ActionRef) -> bool> {
    action_graph: &'a SkyframeActionGraph<'a>,
    target_label: Label,
    target_configuration_checksum: Option<String>,
    actions: Vec<ActionRef>,
    action_mnemonic_matcher: F,
    visited_black_nodes: HashSet<usize>,
    visited_white_nodes: HashSet<Artifact>,
}

/// A node of the visitation: Java recurses `white` -> `visitBlackNode` -> `black` ->
/// `visitWhiteNode`; the recursion is a depth-first walk with an explicit stack here, in the
/// same order.
enum Visit {
    White(Artifact),
    Black(ActionRef),
}

impl<'a, F: Fn(&ActionRef) -> bool> PrintActionVisitor<'a, F> {
    /// Creates a new visitor for the actions associated with the given target that have a
    /// matching mnemonic.
    // Java: com/google/devtools/build/lib/analysis/PrintActionVisitor.java:38 PrintActionVisitor.<init>
    fn new(
        action_graph: &'a SkyframeActionGraph<'a>,
        target: &ConfiguredTarget,
        action_mnemonic_matcher: F,
    ) -> Self {
        PrintActionVisitor {
            action_graph,
            target_label: target.get_label(),
            target_configuration_checksum: target.get_configuration_checksum(),
            actions: Vec::new(),
            action_mnemonic_matcher,
            visited_black_nodes: HashSet::new(),
            visited_white_nodes: HashSet::new(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/PrintActionVisitor.java:47 PrintActionVisitor.shouldVisit
    fn should_visit_action(&self, action: &ActionRef) -> bool {
        let owner = action.get_owner();
        owner.get_label() == Some(&self.target_label)
            && self.target_configuration_checksum.as_deref()
                == Some(owner.get_configuration_checksum())
    }

    // Java: com/google/devtools/build/lib/analysis/PrintActionVisitor.java:54 PrintActionVisitor.visitAction
    // Java: com/google/devtools/build/lib/actions/ActionGraphVisitor.java:41 ActionGraphVisitor.visitAction
    fn visit_action(&mut self, action: &ActionRef) {
        if (self.action_mnemonic_matcher)(action) {
            self.actions.push(action.clone());
        }
    }

    /// Retrieves the collected actions.
    // Java: com/google/devtools/build/lib/analysis/PrintActionVisitor.java:62 PrintActionVisitor.getActions
    fn get_actions(&self) -> Vec<ActionRef> {
        self.actions.clone()
    }

    /// Visit all specified white nodes.
    // Java: com/google/devtools/build/lib/actions/BipartiteVisitor.java:75 BipartiteVisitor.visitWhiteNodes
    fn visit_white_nodes(&mut self, white_nodes: &[Artifact]) {
        for white_node in white_nodes {
            self.walk(Visit::White(white_node.clone()));
        }
    }

    /// `visitWhiteNode` / `visitBlackNode` and their hooks `white` / `black`, depth first.
    // Java: com/google/devtools/build/lib/actions/BipartiteVisitor.java:61 BipartiteVisitor.visitWhiteNode
    // Java: com/google/devtools/build/lib/actions/BipartiteVisitor.java:40 BipartiteVisitor.visitBlackNode
    // Java: com/google/devtools/build/lib/actions/ActionGraphVisitor.java:70 ActionGraphVisitor.white
    // Java: com/google/devtools/build/lib/actions/ActionGraphVisitor.java:79 ActionGraphVisitor.black
    fn walk(&mut self, start: Visit) {
        // Each frame: the remaining children of a black node to visit (white nodes), or a white
        // node whose black node is still to be visited.
        let mut stack: Vec<VecDeque<Visit>> = vec![VecDeque::from([start])];
        while let Some(frame) = stack.last_mut() {
            let Some(node) = frame.pop_front() else {
                stack.pop();
                continue;
            };
            match node {
                Visit::White(artifact) => {
                    if !self.visited_white_nodes.insert(artifact.clone()) {
                        continue;
                    }
                    // white(artifact)
                    let action = self.action_graph.get_generating_action(&artifact);
                    // visitArtifact(artifact): no-op hook.
                    if let Some(action) = action
                        && self.should_visit_action(&action)
                    {
                        stack.push(VecDeque::from([Visit::Black(action)]));
                    }
                }
                Visit::Black(action) => {
                    let id = Arc::as_ptr(&action) as *const () as usize;
                    if !self.visited_black_nodes.insert(id) {
                        continue;
                    }
                    // black(action)
                    self.visit_action(&action);
                    // shouldVisit(Artifact) is always true.
                    let inputs: VecDeque<Visit> = action
                        .get_inputs()
                        .to_list()
                        .iter()
                        .cloned()
                        .map(Visit::White)
                        .collect();
                    stack.push(inputs);
                }
            }
        }
    }
}

/// A stateful filter that keeps track of which files have already been covered. This makes it
/// such that blaze only prints out one action protobuf per file. This is important for headers.
/// In addition, this also handles C++ header files, which are not considered to be action inputs
/// by blaze (due to include scanning).
///
/// As caveats, this only works for files that are given as proper relative paths, rather than
/// using target syntax, and only if the current working directory is the client root.
// Java: com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java:345 PrintActionCommand.ActionFilter
struct ActionFilter<'a> {
    files_desired: &'a mut indexmap::IndexSet<String>,
    runner: &'a PrintActionRunner,
}

impl ActionFilter<'_> {
    // Java: com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java:355 PrintActionCommand.ActionFilter.shouldOutput
    fn should_output(
        &mut self,
        action: Option<&ActionRef>,
        configured_target: &ConfiguredTarget,
        env: &CommandEnvironment,
        skyframe_executor: &SkyframeExecutor,
    ) -> Result<bool, GatherError> {
        let Some(action) = action else {
            return Ok(false);
        };
        // Check all the inputs for the configured target against the file we want argv for.
        let mut artifacts: indexmap::IndexSet<Artifact> = indexmap::IndexSet::new();
        artifacts.extend(action.get_inputs().to_list().iter().cloned());
        artifacts.extend(
            action
                .get_scheduling_dependencies()
                .to_list()
                .iter()
                .cloned(),
        );

        for input in &artifacts {
            if self
                .files_desired
                .shift_remove(input.get_root_relative_path().get_safe_path_string())
            {
                return Ok(self.runner.action_mnemonic_matcher(action));
            }
        }

        // C++ header files show up in the dependency on the Target, but not the
        // ConfiguredTarget, so we also check the target's header files there.
        let rule = match skyframe_executor
            .get_package_manager()
            .get_target(env.get_reporter().clone(), &configured_target.get_label())
        {
            Ok(Target::Rule(rule)) => rule,
            Ok(other) => panic!(
                "ClassCastException: {} cannot be cast to Rule",
                other.get_label()
            ),
            Err(NoSuchThingOrInterrupted::Interrupted) => return Err(GatherError::Interrupted),
            Err(NoSuchThingOrInterrupted::NoSuchThing(_)) => {
                env.get_reporter()
                    .handle(&Event::error("Failed to find target to determine output."));
                return Ok(false);
            }
        };
        use blaze_packages::attribute::attribute_map::AttributeMap as _;
        use blaze_packages::target::rule_or_macro_instance::RuleOrMacroInstance as _;
        use blaze_packages::types::Type;
        if !rule.is_attr_defined("hdrs", Type::LabelList) {
            return Ok(false);
        }

        let ConfiguredTarget::Rule(rule_configured_target) = configured_target else {
            // configuredTarget.getConfigConditions() of the other kinds.
            blaze_util::unported!(
                "ConfiguredTarget.getConfigConditions",
                "{} is not a rule configured target",
                configured_target.get_label()
            );
        };
        let hdrs = blaze_analysis::config::configured_attribute_mapper::ConfiguredAttributeMapper::of_hash(
            rule.clone(),
            rule_configured_target.get_config_conditions().clone(),
            configured_target
                .get_configuration_checksum()
                .unwrap_or_else(|| "null".to_string()),
            /* always_succeed= */ false,
        )
        .get("hdrs", Type::LabelList);
        if let Some(blaze_packages::types::AttrValue::LabelList(hdrs)) = hdrs {
            for hdr_label in hdrs {
                if self
                    .files_desired
                    .shift_remove(hdr_label.to_path_fragment().get_path_string())
                {
                    return Ok(self.runner.action_mnemonic_matcher(action));
                }
            }
        }
        Ok(false) // no match
    }
}

/// `InterruptedFailureDetails.detailedExitCode(message)`.
// Java: com/google/devtools/build/lib/util/InterruptedFailureDetails.java:30 InterruptedFailureDetails.detailedExitCode
fn interrupted_detailed_exit_code(message: &str) -> DetailedExitCode {
    DetailedExitCode::of(FailureDetail {
        message: message.to_string(),
        category: Some(Category::Interrupted(failure_details::Interrupted {
            code: failure_details::interrupted::Code::Interrupted as i32,
        })),
    })
}
