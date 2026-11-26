//! Ports of `analysis/ExtraActionUtils.java` and `analysis/ExtraActionsVisitor.java`.

use std::sync::Arc;

use blaze_actions::{ActionRef, Artifact};
use blaze_collect::NestedSetBuilder;
use blaze_collect::command_line_item::InterruptedException;
use blaze_packages::attribute::AttributeMap as _;
use indexmap::IndexMap;

use super::extra_action_map_provider::ExtraActionMapProvider;
use super::extra_action_spec::ExtraActionSpec;
use crate::configuredtargets::extra_action_artifacts_provider::{
    self, ExtraActionArtifactsProvider,
};
use crate::rule::RuleContext;
use crate::rule::config_util;
use crate::rule::rule_error_consumer::RuleErrorConsumer as _;

/// `Multimap<String, ExtraActionSpec>` built by `computeMnemonicsToExtraActionMap`.
///
/// PORT: java-hash-order. Java's `HashMultimap` keeps a hash set of specs per mnemonic; an
/// `ExtraActionSpec` has identity equality and hash, so the iteration order of several specs for
/// one mnemonic varies from run to run in Bazel. Blaze keeps the first-insertion order and drops
/// a spec already present (identity), as the set does.
type MnemonicToExtraActionMap = IndexMap<String, Vec<ExtraActionSpec>>;

/// Scans `action_listeners` associated with this build to see if any `extra_actions` should be
/// added to this configured target. If any action_listeners are present, a partial visit of the
/// artifact/action graph is performed (for as long as actions found are owned by this
/// `ConfiguredTarget`). Any actions that match the `action_listener` get an `extra_action`
/// associated. The output artifacts of the extra_action are reported to the
/// `AnalysisEnvironment` for bookkeeping.
///
/// `actions_without_extra_action`: compared by identity, as Java's `Set<ActionAnalysisMetadata>`
/// of actions (no action overrides `equals`).
// Java: com/google/devtools/build/lib/analysis/ExtraActionUtils.java:41 ExtraActionUtils.createExtraActionProvider
pub fn create_extra_action_provider(
    actions_without_extra_action: &[ActionRef],
    rule_context: &RuleContext,
) -> Result<ExtraActionArtifactsProvider, InterruptedException> {
    let configuration = rule_context.get_configuration();
    if configuration.is_tool_configuration() {
        return Ok(extra_action_artifacts_provider::EMPTY.clone());
    }

    let mut extra_action_artifacts: Vec<Artifact> = Vec::new();
    let mut builder: NestedSetBuilder<Artifact> = NestedSetBuilder::stable_order();

    let action_listener_labels = config_util::get_action_listeners(configuration);
    if !action_listener_labels.is_empty()
        && rule_context
            .attributes()
            .get_attribute_definition(":action_listener")
            .is_some()
    {
        let mut visitor = ExtraActionsVisitor::new(
            rule_context,
            compute_mnemonics_to_extra_action_map(rule_context),
        );

        // The action list is modified within the body of the loop by the maybeAddExtraAction()
        // call, thus the copy
        for action in rule_context
            .get_analysis_environment()
            .get_registered_actions()
        {
            if !actions_without_extra_action
                .iter()
                .any(|a| Arc::ptr_eq(a, &action))
            {
                visitor.maybe_add_extra_action(&action)?;
            }
        }

        extra_action_artifacts = visitor.get_and_reset_extra_artifacts();
        if !extra_action_artifacts.is_empty() {
            builder.add_all(extra_action_artifacts.iter().cloned());
        }
    }

    // Add extra action artifacts from dependencies
    for prerequisite in rule_context.get_all_prerequisites() {
        if let Some(provider) =
            crate::configuredtargets::get_provider::<ExtraActionArtifactsProvider>(&prerequisite)
        {
            builder.add_transitive(provider.get_transitive_extra_action_artifacts());
        }
    }

    let mut direct: NestedSetBuilder<Artifact> = NestedSetBuilder::stable_order();
    direct.add_all(extra_action_artifacts);
    Ok(ExtraActionArtifactsProvider::create(
        direct.build(),
        builder.build(),
    ))
}

/// Populates the configuration specific mnemonicToExtraActionMap based on all action_listers
/// selected by the user (via the blaze option `--experimental_action_listener=<target>`).
// Java: com/google/devtools/build/lib/analysis/ExtraActionUtils.java:92 ExtraActionUtils.computeMnemonicsToExtraActionMap
fn compute_mnemonics_to_extra_action_map(rule_context: &RuleContext) -> MnemonicToExtraActionMap {
    // We copy the multimap here every time. This could be expensive.
    let mut mnemonic_to_extra_action_map: MnemonicToExtraActionMap = IndexMap::new();
    for action_listener in rule_context.get_prerequisites(":action_listener") {
        match crate::configuredtargets::get_provider::<ExtraActionMapProvider>(&action_listener) {
            None => {
                rule_context.rule_error(&format!(
                    "Unable to match experimental_action_listeners to this rule. Specified \
                     target {} is not an action_listener rule",
                    action_listener.get_label()
                ));
            }
            Some(provider) => {
                for (mnemonic, specs) in provider.get_extra_action_map() {
                    let entry = mnemonic_to_extra_action_map
                        .entry(mnemonic.clone())
                        .or_default();
                    for spec in specs {
                        if !entry.iter().any(|s| s.same(spec)) {
                            entry.push(spec.clone());
                        }
                    }
                }
            }
        }
    }
    mnemonic_to_extra_action_map
}

/// A bipartite graph visitor which accumulates extra actions for a target.
///
/// PORT: only `maybeAddExtraAction` is ported: `createExtraActionProvider` never starts the
/// graph visitation (`ActionGraphVisitor.visitWhiteNodes`), so `visitAction` and the action
/// graph of the analysis environment are not used.
// Java: com/google/devtools/build/lib/analysis/ExtraActionsVisitor.java:33 ExtraActionsVisitor
struct ExtraActionsVisitor<'a> {
    rule_context: &'a RuleContext,
    mnemonic_to_extra_action_map: MnemonicToExtraActionMap,
    extra_artifacts: Vec<Artifact>,
}

impl<'a> ExtraActionsVisitor<'a> {
    /// Creates a new visitor for the extra actions associated with the given target.
    // Java: com/google/devtools/build/lib/analysis/ExtraActionsVisitor.java:39 ExtraActionsVisitor.<init>
    fn new(
        rule_context: &'a RuleContext,
        mnemonic_to_extra_action_map: MnemonicToExtraActionMap,
    ) -> Self {
        ExtraActionsVisitor {
            rule_context,
            mnemonic_to_extra_action_map,
            extra_artifacts: Vec::new(),
        }
    }

    /// `original instanceof Action`: every Blaze action is one.
    // Java: com/google/devtools/build/lib/analysis/ExtraActionsVisitor.java:47 ExtraActionsVisitor.maybeAddExtraAction
    fn maybe_add_extra_action(&mut self, original: &ActionRef) -> Result<(), InterruptedException> {
        if let Some(extra_actions) = self
            .mnemonic_to_extra_action_map
            .get(&original.get_mnemonic())
        {
            for extra_action in extra_actions {
                self.extra_artifacts
                    .extend(extra_action.add_extra_action(self.rule_context, original)?);
            }
        }
        Ok(())
    }

    /// Retrieves the collected artifacts since this method was last called and clears the list.
    // Java: com/google/devtools/build/lib/analysis/ExtraActionsVisitor.java:65 ExtraActionsVisitor.getAndResetExtraArtifacts
    fn get_and_reset_extra_artifacts(&mut self) -> Vec<Artifact> {
        std::mem::take(&mut self.extra_artifacts)
    }
}
