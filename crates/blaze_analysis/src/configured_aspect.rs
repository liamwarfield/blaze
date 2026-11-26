//! Port of `analysis/ConfiguredAspect.java` (the interface with its `Builder`,
//! `BasicConfiguredAspect` and `NonApplicableAspect`).

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, LazyLock};

use blaze_actions::{ActionRef, Artifact};
use blaze_collect::{NestedSet, NestedSetBuilder};
use blaze_packages::rule_class::ProviderKey;
use blaze_packages::starlark_defs::builtin_providers::OUTPUT_GROUP_INFO;
use blaze_packages::starlark_defs::provider::StarlarkProvider;
use blaze_starlark::structs::StarlarkInfo;
use blaze_util::check_state;
use starlark::values::OwnedFrozenValue;

use crate::aspect_value::AspectValue;
use crate::info::info_provider_key;
use crate::provider_map::{
    TransitiveInfoProvider, TransitiveInfoProviderMap, TransitiveInfoProviderMapBuilder,
};
use crate::providers::output_group_info::{self, OutputGroupInfo};
use crate::providers::required_config_fragments_provider::RequiredConfigFragmentsProvider;
use crate::rule::actions_util::{self, AssignOwnersError};
use crate::rule::config_util;
use crate::rule::rule_configured_target_builder::RuleConfiguredTargetBuildError;
use crate::rule::rule_context::RuleContext;
use crate::rule::rule_error_consumer::RuleErrorConsumer;

/// Java `OutputGroupInfo.STARLARK_NAME`.
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:63 OutputGroupInfo.STARLARK_NAME
const OUTPUT_GROUP_INFO_STARLARK_NAME: &str = "output_groups";

/// Java `TransitiveInfoProviderMapImpl.empty()` / `new TransitiveInfoProviderMapBuilder().build()`.
static EMPTY_PROVIDERS: LazyLock<TransitiveInfoProviderMap> =
    LazyLock::new(TransitiveInfoProviderMap::default);

/// Extra information about a configured target computed on request of a dependent.
///
/// Analogous to `ConfiguredTarget`: contains a bunch of transitive info providers, which are
/// merged with the providers of the associated configured target before they are passed to the
/// configured target factories that depend on the configured target to which this aspect is
/// added.
///
/// Aspects are created alongside configured targets on request from dependents.
///
/// PORT: Java's interface has three implementations, which are this enum's variants:
/// `BasicConfiguredAspect`, the `NonApplicableAspect.INSTANCE` singleton (Java compares it with
/// `==`; here [`ConfiguredAspect::is_non_applicable`] matches only that variant), and
/// `AspectValue` (which implements `ConfiguredAspect` in Java; dependents merge `AspectValue`s).
// Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:54 ConfiguredAspect
#[derive(Clone)]
pub enum ConfiguredAspect {
    /// Java `BasicConfiguredAspect`.
    Basic(Arc<BasicConfiguredAspect>),
    /// Java `NonApplicableAspect.INSTANCE`: an aspect that could not be applied to a target.
    NonApplicable,
    /// Java `AspectValue` (an `AspectValue` is a `ConfiguredAspect`).
    Value(Arc<AspectValue>),
}

impl ConfiguredAspect {
    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:56 ConfiguredAspect.getActions
    pub fn get_actions(&self) -> &[ActionRef] {
        match self {
            ConfiguredAspect::Basic(b) => &b.actions,
            // Java: NonApplicableAspect.ACTIONS
            ConfiguredAspect::NonApplicable => &[],
            ConfiguredAspect::Value(v) => v.get_actions(),
        }
    }

    /// Returns the providers created by the aspect.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:59 ConfiguredAspect.getProviders
    pub fn get_providers(&self) -> &TransitiveInfoProviderMap {
        match self {
            ConfiguredAspect::Basic(b) => &b.providers,
            // Java: NonApplicableAspect.PROVIDERS
            ConfiguredAspect::NonApplicable => &EMPTY_PROVIDERS,
            ConfiguredAspect::Value(v) => v.get_providers(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:63 ConfiguredAspect.getProvider
    pub fn get_provider<P: TransitiveInfoProvider>(&self) -> Option<&P> {
        self.get_providers().get_provider::<P>()
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:69 ConfiguredAspect.get
    pub fn get(&self, key: &ProviderKey) -> Option<OwnedFrozenValue> {
        self.get_providers().get(key).cloned()
    }

    /// Java `get(String legacyKey)`.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:74 ConfiguredAspect.get
    pub fn get_legacy(&self, legacy_key: &str) -> Option<OwnedFrozenValue> {
        if OUTPUT_GROUP_INFO_STARLARK_NAME == legacy_key {
            return self.get(&OUTPUT_GROUP_INFO.key());
        }
        self.get_providers().get_legacy(legacy_key).cloned()
    }

    /// Java `this == NonApplicableAspect.INSTANCE`.
    pub fn is_non_applicable(&self) -> bool {
        matches!(self, ConfiguredAspect::NonApplicable)
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:81 ConfiguredAspect.forAlias
    pub fn for_alias(real: &ConfiguredAspect) -> ConfiguredAspect {
        // Aspect on aliases don't have actions, so don't return the actions of the aliased
        // target. They still propagate providers from the real aspect, though.
        ConfiguredAspect::Basic(Arc::new(BasicConfiguredAspect::new(
            Vec::new(),
            real.get_providers().clone(),
        )))
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:88 ConfiguredAspect.builder
    pub fn builder(rule_context: &RuleContext) -> ConfiguredAspectBuilder<'_> {
        ConfiguredAspectBuilder::new(rule_context)
    }
}

impl fmt::Debug for ConfiguredAspect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfiguredAspect::Basic(b) => fmt::Debug::fmt(&**b, f),
            // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:282 NonApplicableAspect.toString
            ConfiguredAspect::NonApplicable => f.write_str("NonApplicableAspect{}"),
            ConfiguredAspect::Value(v) => fmt::Debug::fmt(&**v, f),
        }
    }
}

/// Builder for [`ConfiguredAspect`].
// Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:93 ConfiguredAspect.Builder
pub struct ConfiguredAspectBuilder<'a> {
    providers: TransitiveInfoProviderMapBuilder,
    /// Java `TreeMap<String, NestedSetBuilder<Artifact>>`.
    output_group_builders: BTreeMap<String, NestedSetBuilder<Artifact>>,
    rule_context: &'a RuleContext,
}

impl<'a> ConfiguredAspectBuilder<'a> {
    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:99 ConfiguredAspect.Builder.<init>
    pub fn new(rule_context: &'a RuleContext) -> ConfiguredAspectBuilder<'a> {
        ConfiguredAspectBuilder {
            providers: TransitiveInfoProviderMapBuilder::default(),
            output_group_builders: BTreeMap::new(),
            rule_context,
        }
    }

    /// Adds a provider to the aspect (keyed by its class).
    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:104 ConfiguredAspect.Builder.addProvider
    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:114 ConfiguredAspect.Builder.addProvider
    pub fn add_provider<P: TransitiveInfoProvider>(&mut self, provider: P) -> &mut Self {
        self.providers.add_native(provider);
        self
    }

    /// Like [`Self::add_provider`] for a shared provider.
    pub fn add_provider_arc<P: TransitiveInfoProvider>(&mut self, provider: Arc<P>) -> &mut Self {
        self.providers.add_native_arc(provider);
        self
    }

    /// Adds a set of files to an output group.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:126 ConfiguredAspect.Builder.addOutputGroup
    pub fn add_output_group(&mut self, name: &str, artifacts: &NestedSet<Artifact>) -> &mut Self {
        self.output_group_builders
            .entry(name.to_owned())
            .or_insert_with(NestedSetBuilder::stable_order)
            .add_transitive(artifacts);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:134 ConfiguredAspect.Builder.addStarlarkTransitiveInfo
    pub fn add_starlark_transitive_info(
        &mut self,
        name: &str,
        value: OwnedFrozenValue,
    ) -> &mut Self {
        self.providers.put_legacy(name, value);
        self
    }

    /// Adds a declared provider returned by a Starlark aspect implementation. `Err` is Java's
    /// `EvalException`.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:140 ConfiguredAspect.Builder.addStarlarkDeclaredProvider
    pub fn add_starlark_declared_provider(
        &mut self,
        declared_provider: OwnedFrozenValue,
    ) -> Result<&mut Self, starlark::Error> {
        // Java: Provider constructor = declaredProvider.getProvider(); constructor.isExported().
        // PORT: builtin providers are always exported (BuiltinProvider.isExported); only a
        // Starlark-defined provider can be unexported.
        let value = declared_provider.value();
        if let Some(info) = StarlarkInfo::from_value(value) {
            if let Some(constructor) = StarlarkProvider::from_value(info.provider()) {
                if !constructor.is_exported() {
                    return Err(starlark::Error::new_other(anyhow::anyhow!(
                        "aspect function returned an instance of a provider (defined at {}) that \
                         is not a global",
                        constructor.get_location()
                    )));
                }
            }
        }
        self.add_declared_provider(declared_provider);
        Ok(self)
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:152 ConfiguredAspect.Builder.addDeclaredProvider
    fn add_declared_provider(&mut self, declared_provider: OwnedFrozenValue) {
        self.providers.put_info(declared_provider);
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:157 ConfiguredAspect.Builder.addNativeDeclaredProvider
    pub fn add_native_declared_provider(
        &mut self,
        declared_provider: OwnedFrozenValue,
    ) -> &mut Self {
        // Java: Preconditions.checkState(constructor.isExported()).
        check_state!(info_provider_key(declared_provider.value()).is_some());
        self.add_declared_provider(declared_provider);
        self
    }

    /// Java `build()`: `Ok(None)` where Java returns null (a rule error was reported).
    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:165 ConfiguredAspect.Builder.build
    pub fn build(mut self) -> Result<Option<ConfiguredAspect>, RuleConfiguredTargetBuildError> {
        let rule_context = self.rule_context;
        if !self.output_group_builders.is_empty() {
            if self.providers.contains_info(&OUTPUT_GROUP_INFO.key()) {
                panic!(
                    "IllegalStateException: OutputGroupInfo was provided explicitly; do not use \
                     addOutputGroup"
                );
            }
            let info = OutputGroupInfo::from_builders(&self.output_group_builders);
            self.add_declared_provider(output_group_info::freeze(info));
        }

        // Only add {@link ExtraActionProvider} if extra action listeners are applied
        if !config_util::get_action_listeners(rule_context.get_configuration()).is_empty() {
            let provider = crate::extra::extra_action_utils::create_extra_action_provider(
                /* actions_without_extra_action= */ &[],
                rule_context,
            )
            .map_err(|_| RuleConfiguredTargetBuildError::Interrupted)?;
            self.add_provider(provider);
        }

        let analysis_environment = rule_context.get_analysis_environment();
        let actions = analysis_environment.get_registered_actions();
        match actions_util::assign_owners_and_throw_if_conflict_tolerating_shared_actions(
            analysis_environment.get_action_key_context(),
            &actions,
            rule_context.get_owner(),
        ) {
            Ok(()) => {}
            Err(AssignOwnersError::ArtifactGeneratedByOtherRule(m)) => {
                rule_context.rule_error(&m);
                return Ok(None);
            }
            Err(AssignOwnersError::ActionConflict(m)) => {
                return Err(RuleConfiguredTargetBuildError::ActionConflict(m));
            }
            Err(AssignOwnersError::Interrupted) => {
                return Err(RuleConfiguredTargetBuildError::Interrupted);
            }
        }

        self.maybe_add_required_config_fragments_provider();

        let provider_map = self.providers.build();

        // PORT: StarlarkApiProvider.init (legacy struct-field providers of native rules) has no
        // ported instances.

        if actions.is_empty() && provider_map.get_provider_count() == 0 {
            // Java: BasicConfiguredAspect.EMPTY
            return Ok(Some(ConfiguredAspect::Basic(Arc::new(
                BasicConfiguredAspect::new(Vec::new(), TransitiveInfoProviderMap::default()),
            ))));
        }
        Ok(Some(ConfiguredAspect::Basic(Arc::new(
            BasicConfiguredAspect::new(actions, provider_map),
        ))))
    }

    /// Adds `RequiredConfigFragmentsProvider` if `CoreOptions#includeRequiredConfigFragmentsProvider`
    /// isn't `OFF` and if the provider was not already added.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:218 ConfiguredAspect.Builder.maybeAddRequiredConfigFragmentsProvider
    fn maybe_add_required_config_fragments_provider(&mut self) {
        if self
            .rule_context
            .should_include_required_config_fragments_provider()
            && !self
                .providers
                .contains_native::<RequiredConfigFragmentsProvider>()
        {
            // Java: addProvider(ruleContext.getRequiredConfigFragments()) (checkNotNull).
            let provider = self
                .rule_context
                .get_required_config_fragments()
                .expect("NullPointerException: RequiredConfigFragmentsProvider");
            self.add_provider(provider);
        }
    }

    /// The rule context this builder builds for.
    pub fn get_rule_context(&self) -> &'a RuleContext {
        self.rule_context
    }
}

/// Basic implementation of [`ConfiguredAspect`].
// Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:227 ConfiguredAspect.BasicConfiguredAspect
pub struct BasicConfiguredAspect {
    actions: Vec<ActionRef>,
    providers: TransitiveInfoProviderMap,
}

impl BasicConfiguredAspect {
    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:236 ConfiguredAspect.BasicConfiguredAspect.<init>
    fn new(actions: Vec<ActionRef>, providers: TransitiveInfoProviderMap) -> Self {
        BasicConfiguredAspect { actions, providers }
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:243 ConfiguredAspect.BasicConfiguredAspect.getActions
    pub fn get_actions(&self) -> &[ActionRef] {
        &self.actions
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:248 ConfiguredAspect.BasicConfiguredAspect.getProviders
    pub fn get_providers(&self) -> &TransitiveInfoProviderMap {
        &self.providers
    }
}

impl fmt::Debug for BasicConfiguredAspect {
    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspect.java:253 ConfiguredAspect.BasicConfiguredAspect.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "BasicConfiguredAspect{{actions={:?}, providers={:?}}}",
            self.actions, self.providers
        )
    }
}
