//! Ports of `query2/cquery/TransitionsOutputFormatterCallback.java` (cquery
//! `--output=transitions`) and the formatting half of `query2/cquery/CqueryTransitionResolver.java`
//! (the dependency resolution half is `blaze_sky_analysis::cquery_transition_resolver`).

use std::sync::Arc;

use blaze_analysis::config::transitions::{
    ConfigurationTransition, RuleTransitionData, create_rule_transition,
    create_rule_transition_with_trimming,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_and_data::ConfiguredTargetAndData;
use blaze_analysis::dependency_kind::DependencyKind;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::build_options::BuildOptions;
use blaze_build_config::options_diff::OptionsDiff;
use blaze_cmdline::Label;
use blaze_events::Event;
use blaze_optdefs::command::cquery_options::Transitions;
use blaze_packages::target::{LabelPrinter, Target};
use blaze_query_engine::callback::{
    IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::{
    Callback, OutputFormatterCallback, QueryError, ThreadSafeOutputFormatterCallback,
};
use blaze_sky_analysis::cquery_transition_resolver::{
    CqueryTransitionResolverKey, CqueryTransitionResolverValue,
};
use blaze_sky_analysis::producers::configured_aspect_producer::configured_target_key_from_configured_target;
use blaze_sky_top::skyframe_executor::{DEFAULT_THREAD_COUNT, SkyframeExecutor};
use blaze_skyframe::SkyKey;
use indexmap::IndexMap;

use crate::cquery::configured_target_query_environment::CqueryNode;
use crate::cquery::cquery_threadsafe_callback::{CqueryThreadsafeCallback, short_id};
use crate::post_analysis_query_environment::NamedThreadSafeOutputFormatterCallback;

/// ResolvedTransition represents a single edge in the dependency graph, between some target and a
/// target it depends on, reachable via a single attribute.
// Java: com/google/devtools/build/lib/query2/cquery/CqueryTransitionResolver.java:56 CqueryTransitionResolver.ResolvedTransition
#[derive(Clone, PartialEq, Eq)]
struct ResolvedTransition {
    /// The label of the target being depended on.
    label: Label,
    /// The configuration(s) this edge results in. This is a collection because a split
    /// transition may lead to a single attribute requesting a dependency in multiple
    /// configurations. If no transition was applied to an attribute, this collection is empty.
    options: Vec<Arc<BuildOptions>>,
    /// The name of the attribute via which the dependency was requested.
    attribute_name: String,
    /// The name of the transition applied to the attribute.
    transition_name: String,
}

/// What `CqueryTransitionResolver.dependencies` throws.
enum DependenciesError {
    /// `EvaluateException` (its message).
    Evaluate(String),
    /// `IncompatibleTargetChecker.IncompatibleTargetException`.
    IncompatibleTarget,
    /// `InterruptedException`.
    Interrupted,
}

/// Output formatter that prints `ConfigurationTransition` information for rule configured
/// targets in the results of a cquery call.
// Java: com/google/devtools/build/lib/query2/cquery/TransitionsOutputFormatterCallback.java:51 TransitionsOutputFormatterCallback
pub struct TransitionsOutputFormatterCallback<'a> {
    base: CqueryThreadsafeCallback<'a>,
    skyframe_executor: &'a SkyframeExecutor,
    label_printer: LabelPrinter,
    /// Whether a CQUERY_TRANSITION_RESOLVER node was evaluated (see
    /// [`Self::delete_resolver_nodes`]).
    resolved_any: std::cell::Cell<bool>,
}

impl<'a> TransitionsOutputFormatterCallback<'a> {
    // Java: com/google/devtools/build/lib/query2/cquery/TransitionsOutputFormatterCallback.java:66 TransitionsOutputFormatterCallback.<init>
    pub fn new(
        base: CqueryThreadsafeCallback<'a>,
        skyframe_executor: &'a SkyframeExecutor,
        label_printer: LabelPrinter,
    ) -> Self {
        TransitionsOutputFormatterCallback {
            base,
            skyframe_executor,
            label_printer,
            resolved_any: std::cell::Cell::new(false),
        }
    }

    /// Removes the CQUERY_TRANSITION_RESOLVER nodes from the graph once the output is done.
    ///
    /// PORT: Java's resolver runs outside Skyframe and leaves nothing in the graph; Blaze's
    /// nodes are deleted here (marked, then removed by an evaluation of no roots, which applies
    /// pending deletions), so they neither count in the graph size the command reports
    /// (`post_invocation_skyframe_node_count`) nor survive into later commands. The evaluation
    /// statistics of the command still count their evaluations (docs/reviews/
    /// 2026-11-26-query-2-response.md, S2-2).
    fn delete_resolver_nodes(&self) -> Result<(), OutputFormatterError> {
        self.skyframe_executor.get_evaluator().delete(|key, _| {
            key.function_name() == <CqueryTransitionResolverKey as blaze_skyframe::Key>::FUNCTION
        });
        match self.skyframe_executor.evaluate(
            &[],
            /* keep_going= */ false,
            DEFAULT_THREAD_COUNT,
            self.base.event_handler.clone(),
        ) {
            Ok(_) => Ok(()),
            Err(blaze_skyframe::Interrupted) => Err(OutputFormatterError::Interrupted(
                blaze_query_engine::InterruptedException::new(),
            )),
        }
    }

    // Java: com/google/devtools/build/lib/query2/cquery/TransitionsOutputFormatterCallback.java:144 TransitionsOutputFormatterCallback.getRuleClassTransition
    fn get_rule_class_transition(ct: &CqueryNode, target: &Target) -> String {
        let Some(rule) = target.get_associated_rule() else {
            return String::new();
        };
        let factory = rule.get_rule_class_object().get_transition_factory();
        let options_checksum = ct
            .get_configuration_key()
            .expect("NullPointerException: getConfigurationKey()")
            .get_options_checksum()
            .to_owned();
        let transition = create_rule_transition(
            factory,
            &RuleTransitionData::create(
                rule.clone(),
                /* configConditions= */ Arc::new(IndexMap::new()),
                options_checksum,
            ),
        );
        format!("{} -> ", transition.get_name())
    }

    /// Return the set of dependencies of a ConfiguredTarget, including information about the
    /// configuration transitions applied to the dependencies.
    // Java: com/google/devtools/build/lib/query2/cquery/CqueryTransitionResolver.java:130 CqueryTransitionResolver.dependencies
    fn dependencies(
        &self,
        configured_target: &CqueryNode,
    ) -> Result<Vec<ResolvedTransition>, DependenciesError> {
        let CqueryNode::ConfiguredTarget(ConfiguredTarget::Rule(_)) = configured_target else {
            return Ok(Vec::new());
        };
        let ct = configured_target
            .as_configured_target()
            .expect("a configured target");

        let configuration = self
            .base
            .get_configuration(configured_target.get_configuration_key().as_ref());

        let key = CqueryTransitionResolverKey(configured_target_key_from_configured_target(ct));
        // Java's DependencyResolver reads a lookup environment over the graph analysis left
        // and computes nothing: a value it requests that is not done makes `evaluate` return
        // false. Blaze's evaluation of CQUERY_TRANSITION_RESOLVER computes such a value
        // instead, so a node computed besides the resolver's own one is that case.
        let evaluator = self.skyframe_executor.get_evaluator();
        let computed_before = evaluator.computed_count();
        let result = match self.skyframe_executor.evaluate(
            &[SkyKey::new(key.clone())],
            /* keep_going= */ false,
            DEFAULT_THREAD_COUNT,
            self.base.event_handler.clone(),
        ) {
            Ok(result) => result,
            Err(blaze_skyframe::Interrupted) => return Err(DependenciesError::Interrupted),
        };
        self.resolved_any.set(true);
        if evaluator.computed_count() - computed_before > 1 {
            // Java: com/google/devtools/build/lib/query2/cquery/CqueryTransitionResolver.java:147 CqueryTransitionResolver.dependencies
            return Err(DependenciesError::Evaluate(
                "DependencyResolver.evaluate did not complete".to_owned(),
            ));
        }
        let Some(value) = result.get(&key) else {
            blaze_util::unported!(
                "CqueryTransitionResolver.dependencies",
                "no value for {:?}: {:?}",
                key,
                result.get_error(&SkyKey::new(key.clone()))
            );
        };
        let (deps, transitions) = match &*value {
            CqueryTransitionResolverValue::Resolved {
                dep_value_map,
                transitions,
            } => (dep_value_map, transitions),
            CqueryTransitionResolverValue::IncompatibleTarget => {
                return Err(DependenciesError::IncompatibleTarget);
            }
            CqueryTransitionResolverValue::Error(message) => {
                return Err(DependenciesError::Evaluate(message.clone()));
            }
        };

        // ImmutableSet.builder(): insertion order, duplicates dropped.
        let mut resolved: Vec<ResolvedTransition> = Vec::new();
        let mut add = |t: ResolvedTransition| {
            if !resolved.contains(&t) {
                resolved.push(t);
            }
        };
        for (kind, targets) in deps {
            if matches!(kind, DependencyKind::NonAttribute(_)) {
                continue; // No attribute edge to report.
            }

            // There can be multiple labels under a given kind. Groups the targets by label
            // (Multimaps.index: keys in order of first appearance).
            let mut targets_by_label: IndexMap<Label, Vec<&ConfiguredTargetAndData>> =
                IndexMap::new();
            for prerequisite in targets {
                targets_by_label
                    .entry(prerequisite.get_configured_target().get_original_label())
                    .or_default()
                    .push(prerequisite);
            }
            let dependency_name = Self::get_dependency_name(kind);
            // attributeTransitionCollector.row(kind): HashBasedTable.put keeps the last value.
            let attribute_transition_of = |label: &Label| -> Option<&ConfigurationTransition> {
                transitions
                    .iter()
                    .rev()
                    .find(|(k, l, _)| k == kind && l == label)
                    .map(|(_, _, t)| t)
            };

            for (label, targets) in &targets_by_label {
                // The most common case, so short-circuit this.
                if let Some(transition_name) =
                    Self::uses_no_transition(configuration.as_deref(), targets)
                {
                    add(ResolvedTransition {
                        label: label.clone(),
                        options: Vec::new(),
                        attribute_name: dependency_name.clone(),
                        transition_name,
                    });
                    continue;
                }

                // The rule transition does not vary across a split so using the first target is
                // sufficient.
                let rule_transition = self.get_rule_transition(targets[0]);

                let to_options: Vec<Arc<BuildOptions>> = targets
                    .iter()
                    .map(|t| {
                        t.get_configuration()
                            .expect("a transitioned dependency has a configuration")
                            .get_options()
                            .clone()
                    })
                    .collect();

                add(ResolvedTransition {
                    label: label.clone(),
                    options: to_options,
                    attribute_name: dependency_name.clone(),
                    transition_name: Self::get_transition_name(
                        attribute_transition_of(label),
                        rule_transition.as_ref(),
                    ),
                });
            }
        }
        Ok(resolved)
    }

    // Java: com/google/devtools/build/lib/query2/cquery/CqueryTransitionResolver.java:217 CqueryTransitionResolver.usesNoTransition
    fn uses_no_transition(
        from_configuration: Option<&BuildConfigurationValue>,
        targets: &[&ConfiguredTargetAndData],
    ) -> Option<String> {
        let first = targets[0];
        // Check whether the configuration changed.
        if targets.len() == 1 && from_configuration == first.get_configuration().map(|c| &**c) {
            return Some(ConfigurationTransition::NoTransition.get_name());
        }
        // If any target has a null configuration, they all do, so it's sufficient to check the
        // first.
        if first.get_configuration().is_none() {
            return Some("(null transition)".to_owned());
        }
        None
    }

    // Java: com/google/devtools/build/lib/query2/cquery/CqueryTransitionResolver.java:232 CqueryTransitionResolver.getDependencyName
    fn get_dependency_name(kind: &DependencyKind) -> String {
        match kind {
            DependencyKind::Toolchain {
                exec_group_name,
                is_default_exec_group,
            }
            | DependencyKind::BaseTargetToolchain {
                exec_group_name,
                is_default_exec_group,
                ..
            } => {
                if *is_default_exec_group {
                    return "[toolchain dependency]".to_owned();
                }
                return format!("[toolchain dependency: {exec_group_name}]");
            }
            _ => {}
        }
        kind.get_attribute()
            .expect("an attribute dependency kind")
            .get_name()
            .to_owned()
    }

    /// Keep in sync with TargetAndConfigurationProducer.computeTransition.
    // Java: com/google/devtools/build/lib/query2/cquery/CqueryTransitionResolver.java:244 CqueryTransitionResolver.getRuleTransition
    fn get_rule_transition(
        &self,
        configured_target: &ConfiguredTargetAndData,
    ) -> Option<ConfigurationTransition> {
        // accessor.getTarget(configuredTarget): the target of its original label (an alias's
        // own rule, not the actual target).
        let target: Target = self
            .base
            .accessor
            .get_accessor_target(&CqueryNode::ConfiguredTarget(
                configured_target.get_configured_target().clone(),
            ));
        let rule = target.get_associated_rule()?;
        let transition_factory = rule.get_rule_class_object().get_transition_factory();
        // ((ConfiguredRuleClassProvider) ruleClassProvider).getTrimmingTransitionFactory() is
        // never null in Bazel.
        let is_alias = rule.get_name() == "alias";
        let transition_data = RuleTransitionData::create(
            rule.clone(),
            /* configConditions= */ Arc::new(IndexMap::new()),
            String::new(),
        );
        if !is_alias {
            // ComposingTransitionFactory.ofUnchecked(transitionFactory, trimmingTransitionFactory)
            Some(create_rule_transition_with_trimming(
                transition_factory,
                &transition_data,
            ))
        } else {
            Some(create_rule_transition(transition_factory, &transition_data))
        }
    }

    // Java: com/google/devtools/build/lib/query2/cquery/CqueryTransitionResolver.java:265 CqueryTransitionResolver.getTransitionName
    fn get_transition_name(
        attribute_transition: Option<&ConfigurationTransition>,
        rule_transition: Option<&ConfigurationTransition>,
    ) -> String {
        let rule_transition =
            rule_transition.expect("NullPointerException: getRuleTransition returned null");
        match attribute_transition {
            None => rule_transition.get_name(),
            Some(a) if a.is_no_transition() => rule_transition.get_name(),
            Some(a) if rule_transition.is_no_transition() => a.get_name(),
            Some(a) => format!("({} + {})", a.get_name(), rule_transition.get_name()),
        }
    }
}

impl NamedThreadSafeOutputFormatterCallback<CqueryNode> for TransitionsOutputFormatterCallback<'_> {
    // Java: com/google/devtools/build/lib/query2/cquery/TransitionsOutputFormatterCallback.java:58 TransitionsOutputFormatterCallback.getName
    fn get_name(&self) -> String {
        "transitions".to_owned()
    }
}

impl ThreadSafeOutputFormatterCallback<CqueryNode> for TransitionsOutputFormatterCallback<'_> {}

impl Callback<CqueryNode> for TransitionsOutputFormatterCallback<'_> {
    fn process(&self, partial_result: &[CqueryNode]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<CqueryNode> for TransitionsOutputFormatterCallback<'_> {
    // Java: com/google/devtools/build/lib/query2/cquery/TransitionsOutputFormatterCallback.java:82 TransitionsOutputFormatterCallback.processOutput
    fn process_output(&self, partial_result: &[CqueryNode]) -> Result<(), OutputFormatterError> {
        let verbosity = self.base.options.transitions;
        if verbosity == Transitions::NONE {
            self.base.event_handler.handle(&Event::error(
                "Instead of using --output=transitions, set the --transitions flag explicitly to \
                 'lite' or 'full'",
            ));
            return Ok(());
        }
        // partialResultMap.put(kct.getOriginalLabel(), accessor.getTarget(kct))
        let mut partial_result_map: std::collections::HashMap<Label, Target> =
            std::collections::HashMap::new();
        for kct in partial_result {
            partial_result_map.insert(
                kct.get_original_label(),
                self.base.accessor.get_accessor_target(kct),
            );
        }
        for keyed_configured_target in partial_result {
            let target = &partial_result_map[&keyed_configured_target.get_original_label()];
            let config = self
                .base
                .get_configuration(keyed_configured_target.get_configuration_key().as_ref());
            self.base.add_result(format!(
                "{}{} ({})",
                Self::get_rule_class_transition(keyed_configured_target, target),
                self.label_printer
                    .to_string(&keyed_configured_target.get_original_label()),
                short_id(config.as_ref())
            ));
            let dependencies = match self.dependencies(keyed_configured_target) {
                Ok(d) => d,
                Err(DependenciesError::Evaluate(message)) => {
                    // String.format("Failed to evaluate %s: %s", label, e): e.toString() is the
                    // exception's class name and message.
                    self.base.event_handler.handle(&Event::error(format!(
                        "Failed to evaluate {}: com.google.devtools.build.lib.query2.cquery.\
                         CqueryTransitionResolver$EvaluateException: {}",
                        keyed_configured_target.get_original_label(),
                        message
                    )));
                    return Ok(());
                }
                Err(DependenciesError::Interrupted) => {
                    return Err(OutputFormatterError::Interrupted(
                        blaze_query_engine::InterruptedException::new(),
                    ));
                }
                Err(DependenciesError::IncompatibleTarget) => {
                    self.base.event_handler.handle(&Event::warn(format!(
                        "Skipping dependencies of incompatible target {}",
                        keyed_configured_target.get_original_label()
                    )));
                    return Ok(());
                }
            };
            for dep in dependencies {
                self.base.add_result(format!(
                    "  {}#{}#{} -> {}",
                    dep.attribute_name,
                    self.label_printer.to_string(&dep.label),
                    dep.transition_name,
                    dep.options
                        .iter()
                        .map(|o| o.short_id())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                if verbosity == Transitions::LITE {
                    continue;
                }
                let mut diff = OptionsDiff::default();
                let from = config
                    .as_ref()
                    .expect("NullPointerException: config.getOptions()")
                    .get_options();
                for options in &dep.options {
                    OptionsDiff::diff_into(&mut diff, from, options);
                }
                for single_diff in diff.get_pretty_print_list() {
                    self.base.add_result(format!("    {single_diff}"));
                }
            }
        }
        Ok(())
    }

    fn close(&self, fail_fast: bool) -> Result<(), OutputFormatterError> {
        if self.resolved_any.get() {
            self.delete_resolver_nodes()?;
        }
        self.base
            .close(fail_fast)
            .map_err(OutputFormatterError::from)
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.base.io_exception
    }
}

#[cfg(test)]
mod tests {
    // Port of src/test/java/com/google/devtools/build/lib/query2/cquery/TransitionsOutputFormatterTest.java
    // (a ConfiguredTargetQueryTest).

    // Java: src/test/java/com/google/devtools/build/lib/query2/cquery/TransitionsOutputFormatterTest.java:64 TransitionsOutputFormatterTest.testTransitions_full
    #[test]
    #[ignore = "needs the ConfiguredTargetQueryHelper analysis harness"]
    fn test_transitions_full() {}

    // Java: src/test/java/com/google/devtools/build/lib/query2/cquery/TransitionsOutputFormatterTest.java:115 TransitionsOutputFormatterTest.testTransitions_lite
    #[test]
    #[ignore = "needs the ConfiguredTargetQueryHelper analysis harness"]
    fn test_transitions_lite() {}

    // Java: src/test/java/com/google/devtools/build/lib/query2/cquery/TransitionsOutputFormatterTest.java:153 TransitionsOutputFormatterTest.testTransitions_getRightConfigurations
    #[test]
    #[ignore = "needs the ConfiguredTargetQueryHelper analysis harness"]
    fn test_transitions_get_right_configurations() {}

    // Java: src/test/java/com/google/devtools/build/lib/query2/cquery/TransitionsOutputFormatterTest.java:176 TransitionsOutputFormatterTest.testTransitions_noTransitions
    #[test]
    #[ignore = "needs the ConfiguredTargetQueryHelper analysis harness"]
    fn test_transitions_no_transitions() {}

    // Java: src/test/java/com/google/devtools/build/lib/query2/cquery/TransitionsOutputFormatterTest.java:192 TransitionsOutputFormatterTest.nonAttributeDependencySkipped
    #[test]
    #[ignore = "needs the ConfiguredTargetQueryHelper analysis harness"]
    fn non_attribute_dependency_skipped() {}
}
