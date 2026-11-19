//! Port of `BuildViewForTesting.getRuleContextForTesting` (src/test/java
//! analysis/util/BuildViewForTesting.java): a rule context equivalent to the one that was used to
//! create a configured target.
//!
//! PORT: Java runs the dependency resolver outside a SkyFunction, through
//! `SkyFunctionEnvironmentForTesting`; Blaze's evaluator has no environment outside a
//! SkyFunction, so the same code runs in a test SkyFunction (`RULE_CONTEXT_FOR_TESTING`) that the
//! harness registers in its executor, and the rule context is its value. The configured target is
//! already evaluated, so every dependency it requests is done. The analysis environment is a
//! `CachingAnalysisEnvironment` (Java's Starlark variant, `getRuleContextForStarlark`; Java's
//! `getRuleContext` passes a `StubAnalysisEnvironment`), and its events are stored, not reported
//! to the test's reporter.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_actions::ActionLookupKeyRef;
use blaze_analysis::analysis_services::AnalysisServices;
use blaze_analysis::config::config_conditions::ConfigConditions;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::dependency_kind::DependencyKind;
use blaze_analysis::providers::visibility::public_visibility;
use blaze_analysis::rule::analysis_environment::CachingAnalysisEnvironment;
use blaze_analysis::rule::configured_target_factory::ConfiguredTargetFactory;
use blaze_analysis::rule::rule_context::{RuleContext, RuleContextBuilder};
use blaze_analysis::toolchains::{ResolvedToolchainContext, ToolchainCollection};
use blaze_events::StoredEventHandler;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_sky_analysis::dependency_resolver::{DependencyResolver, DependencyResolverState};
use blaze_sky_analysis::producers::errors::TargetAndConfiguration;
use blaze_sky_loading::package_function::PackageKey;
use blaze_sky_loading::starlark_builtins::StarlarkBuiltinsKey;
use blaze_skyframe::{EvaluatorBuilder, Key, SkyFunctionName};
use indexmap::IndexMap;
use parking_lot::Mutex;
use starlark::values::OwnedFrozenValue;

/// The key of the rule context of a configured target.
///
/// PORT: equality and hash use the configured target key only.
#[derive(Clone)]
pub struct RuleContextForTestingKey {
    pub target_and_configuration: TargetAndConfiguration,
    pub configured_target_key: ConfiguredTargetKey,
}

impl PartialEq for RuleContextForTestingKey {
    fn eq(&self, other: &Self) -> bool {
        self.configured_target_key == other.configured_target_key
    }
}

impl Eq for RuleContextForTestingKey {}

impl Hash for RuleContextForTestingKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.configured_target_key.hash(state);
    }
}

impl fmt::Debug for RuleContextForTestingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RuleContextForTesting({:?})", self.configured_target_key)
    }
}

impl Key for RuleContextForTestingKey {
    type Value = RuleContextForTestingValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName("RULE_CONTEXT_FOR_TESTING");
}

/// The rule context.
pub struct RuleContextForTestingValue {
    pub rule_context: Arc<RuleContext>,
}

/// The analysis services of the harness's current executor (set after each executor is built).
pub type AnalysisServicesSlot = Arc<Mutex<Option<Arc<AnalysisServices>>>>;

/// Registers `RULE_CONTEXT_FOR_TESTING`.
// Java: com/google/devtools/build/lib/analysis/util/BuildViewForTesting.java:354 BuildViewForTesting.getRuleContextForTesting
pub fn register(builder: EvaluatorBuilder, slot: AnalysisServicesSlot) -> EvaluatorBuilder {
    builder.typed_function::<RuleContextForTestingKey, _>(move |key, env| {
        let services = slot
            .lock()
            .clone()
            .expect("the executor's analysis services");
        // Java: initializeDependencyResolverState
        let mut state = DependencyResolverState::new(false);
        state.target_and_configuration = Some(key.target_and_configuration.clone());
        // Java: runDependencyResolver
        let mut producer = DependencyResolver::new(key.target_and_configuration.clone());
        match producer.evaluate(&mut state, &key.configured_target_key, &services, env) {
            Ok(true) => {}
            Ok(false) => {
                if env.values_missing() {
                    return Ok(None);
                }
                panic!(
                    "IllegalStateException: {:?} should be already evaluated",
                    key.configured_target_key
                );
            }
            // Should not be possible for done ConfiguredTarget.
            Err(e) => panic!("IllegalStateException: {e:?}"),
        }

        let prerequisite_map = producer.get_dep_value_map().clone();
        let target = key.target_and_configuration.get_target().clone();
        let target_description = target.to_string();

        let mut resolved_toolchain_context =
            ToolchainCollection::<ResolvedToolchainContext>::builder();
        let mut has_toolchains = false;
        if let Some(unloaded_toolchain_collection) = producer.get_unloaded_toolchain_contexts() {
            has_toolchains = true;
            for (exec_group, unloaded_toolchain_context) in
                unloaded_toolchain_collection.context_map()
            {
                let toolchain_targets = prerequisite_map
                    .get(&DependencyKind::for_exec_group(exec_group))
                    .cloned()
                    .unwrap_or_default();
                let toolchain_context = ResolvedToolchainContext::load(
                    unloaded_toolchain_context,
                    &target_description,
                    &toolchain_targets,
                )
                .unwrap_or_else(|e| panic!("ToolchainException: {e:?}"));
                resolved_toolchain_context.add_context(exec_group, toolchain_context);
            }
        }

        // The analysis environment (Java: the caller's CachingAnalysisEnvironment, built as
        // SkyframeBuildView.createAnalysisEnvironment does).
        let Some(starlark_builtins_value) = env.get_value(StarlarkBuiltinsKey::key(false))? else {
            return Ok(None);
        };
        let Some(package_value) = env.get_value(PackageKey(
            target.get_label().get_package_identifier().clone(),
        ))?
        else {
            return Ok(None);
        };
        let configuration = key
            .target_and_configuration
            .get_configuration()
            .cloned()
            .expect("the configuration of a rule");
        let core_options = configuration.get_options().get::<CoreOptions>();
        let starlark_defined_builtins: IndexMap<String, OwnedFrozenValue> = starlark_builtins_value
            .exported_to_java
            .iter()
            .map(|(name, entry)| (name.clone(), entry.value.clone()))
            .collect();
        let analysis_env = Arc::new(CachingAnalysisEnvironment::new(
            services.artifact_factory.clone(),
            services.action_key_context.clone(),
            ActionLookupKeyRef::new(key.configured_target_key.clone()),
            core_options.is_some_and(|o| o.extended_sanity_checks),
            core_options.is_some_and(|o| o.allow_analysis_failures),
            Arc::new(StoredEventHandler::new()),
            starlark_builtins_value.starlark_semantics.clone(),
            Some(Arc::new(starlark_defined_builtins)),
            services.main_repo_mapping.read().clone(),
            Some(services.get_workspace_status_artifacts().clone()),
            Some(package_value.get_package().clone()),
        ));

        let rule = target.get_associated_rule().expect("a rule target").clone();
        let mut builder = RuleContextBuilder::new(
            analysis_env,
            target.clone(),
            /* aspects= */ Vec::new(),
            configuration,
        );
        builder
            .set_rule_class_provider(services.rule_class_provider.clone())
            .set_configuration_fragment_policy(
                rule.get_rule_class_object()
                    .get_configuration_fragment_policy()
                    .clone(),
            )
            .set_action_owner_symbol(ActionLookupKeyRef::new(key.configured_target_key.clone()))
            .set_mutability("configured target")
            .set_visibility(public_visibility())
            .set_prerequisites(ConfiguredTargetFactory::remove_toolchain_deps(
                &prerequisite_map,
            ))
            .set_config_conditions(ConfigConditions::empty())
            .set_toolchain_contexts(
                has_toolchains.then(|| Arc::new(resolved_toolchain_context.build())),
            );
        if let Some(exec_group_collection_builder) = &state.exec_group_collection_builder {
            builder.set_exec_group_collection_builder(
                exec_group_collection_builder
                    .as_exec_group_collection_builder()
                    .clone(),
            );
        }
        let rule_context = builder
            .unsafe_build()
            .unwrap_or_else(|e| panic!("InvalidExecGroupException: {e:?}"));
        Ok(Some(RuleContextForTestingValue {
            rule_context: Arc::new(rule_context),
        }))
    })
}
