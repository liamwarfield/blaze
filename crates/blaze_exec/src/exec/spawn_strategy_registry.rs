//! Port of `lib/exec/SpawnStrategyRegistry.java` (with its `Builder`), the parts of
//! `DynamicStrategyRegistry` / `RemoteLocalFallbackRegistry` it implements, and the
//! `AbruptExitException` it throws.

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_events::{Event, EventHandler};
use blaze_optdefs::common::regex_filter::RegexFilter;
use blaze_protos::blaze::strategy_policy::StrategyPolicy;
use blaze_protos::failure_details::{
    self, FailureDetail, execution_options::Code as ExecutionOptionsCode, failure_detail,
};
use indexmap::IndexMap;

use crate::actions::spawn_strategy::{SpawnStrategy, same_strategy};
use crate::actions::{ActionExecutionMetadata, Spawn};
use crate::exec::action_execution_context::ActionContextRegistry;
use crate::exec::spawn_strategy_policy::SpawnStrategyPolicy;

/// Port of `util/AbruptExitException` for the registry's errors: an exception carrying a
/// `DetailedExitCode` (here its `FailureDetail`); the message is the failure detail's message.
// Java: com/google/devtools/build/lib/util/AbruptExitException.java:22 AbruptExitException
#[derive(Debug, Clone, PartialEq)]
pub struct AbruptExitException {
    pub failure_detail: FailureDetail,
}

impl AbruptExitException {
    /// `new AbruptExitException(DetailedExitCode.of(failureDetail))`.
    pub fn new(failure_detail: FailureDetail) -> AbruptExitException {
        AbruptExitException { failure_detail }
    }

    /// `getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.failure_detail.message
    }

    /// `getDetailedExitCode().getFailureDetail()`.
    pub fn get_failure_detail(&self) -> &FailureDetail {
        &self.failure_detail
    }
}

impl fmt::Display for AbruptExitException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.get_message())
    }
}

impl std::error::Error for AbruptExitException {}

/// `DynamicStrategyRegistry.DynamicMode`.
// Java: com/google/devtools/build/lib/actions/DynamicStrategyRegistry.java:27 DynamicStrategyRegistry.DynamicMode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DynamicMode {
    Local,
    Remote,
}

/// Registry that collects spawn strategies and rules about their applicability and makes them
/// available for querying through various registry interfaces.
///
/// An instance of this registry can be created using its builder
/// ([`SpawnStrategyRegistryBuilder`]), which is available to Blaze modules during server startup.
// Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:65 SpawnStrategyRegistry
pub struct SpawnStrategyRegistry {
    mnemonic_to_strategies: IndexMap<String, Vec<Arc<dyn SpawnStrategy>>>,
    strategy_regex_filter: StrategyRegexFilter,
    strategy_platform_filter: StrategyPlatformFilter,
    default_strategies: Vec<Arc<dyn SpawnStrategy>>,
    mnemonic_to_remote_dynamic_strategies: IndexMap<String, Vec<Arc<dyn SpawnStrategy>>>,
    mnemonic_to_local_dynamic_strategies: IndexMap<String, Vec<Arc<dyn SpawnStrategy>>>,
    remote_local_fallback_strategy: Option<Arc<dyn SpawnStrategy>>,
}

impl fmt::Debug for SpawnStrategyRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpawnStrategyRegistry")
            .field(
                "default_strategies",
                &crate::actions::spawn_strategy::strategies_to_string(&self.default_strategies),
            )
            .finish_non_exhaustive()
    }
}

impl SpawnStrategyRegistry {
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:80 SpawnStrategyRegistry.<init>
    fn new(
        mnemonic_to_strategies: IndexMap<String, Vec<Arc<dyn SpawnStrategy>>>,
        strategy_regex_filter: StrategyRegexFilter,
        strategy_platform_filter: StrategyPlatformFilter,
        default_strategies: Vec<Arc<dyn SpawnStrategy>>,
        mnemonic_to_remote_dynamic_strategies: IndexMap<String, Vec<Arc<dyn SpawnStrategy>>>,
        mnemonic_to_local_dynamic_strategies: IndexMap<String, Vec<Arc<dyn SpawnStrategy>>>,
        remote_local_fallback_strategy: Option<Arc<dyn SpawnStrategy>>,
    ) -> SpawnStrategyRegistry {
        // PORT: skipped, tracing (Java log).
        SpawnStrategyRegistry {
            mnemonic_to_strategies,
            strategy_regex_filter,
            strategy_platform_filter,
            default_strategies,
            mnemonic_to_remote_dynamic_strategies,
            mnemonic_to_local_dynamic_strategies,
            remote_local_fallback_strategy,
        }
    }

    /// Returns the strategies applying to the given spawn, in priority order.
    ///
    /// Which strategies are returned is based on the precedence as documented on the construction
    /// methods of this registry's builder. If the reason for selecting the context is worth
    /// mentioning to the user, logs a message using the given reporter.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:113 SpawnStrategyRegistry.getStrategies
    pub fn get_strategies(
        &self,
        spawn: &dyn Spawn,
        reporter: Option<&dyn EventHandler>,
    ) -> Vec<Arc<dyn SpawnStrategy>> {
        self.strategy_platform_filter.get_strategies(
            spawn,
            self.get_strategies_for_action(
                spawn.get_resource_owner(),
                &spawn.get_mnemonic(),
                reporter,
            ),
        )
    }

    /// Returns the strategies applying to the given action, in priority order.
    ///
    /// NOTE: This method is public for Blaze, Bazel must use `getStrategies(Spawn, EventHandler)`.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:129 SpawnStrategyRegistry.getStrategies
    pub fn get_strategies_for_action(
        &self,
        resource_owner: &dyn ActionExecutionMetadata,
        mnemonic: &str,
        reporter: Option<&dyn EventHandler>,
    ) -> Vec<Arc<dyn SpawnStrategy>> {
        // Don't override test strategies by --strategy_regexp for backwards compatibility.
        if mnemonic != "TestRunner"
            && let Some(description) = resource_owner.get_progress_message()
        {
            let regex_strategies =
                self.strategy_regex_filter
                    .get_strategies(mnemonic, &description, reporter);
            if !regex_strategies.is_empty() {
                return regex_strategies;
            }
        }
        if let Some(strategies) = self.mnemonic_to_strategies.get(mnemonic) {
            return strategies.clone();
        }
        self.default_strategies.clone()
    }

    /// `notifyUsedDynamic(actionContextRegistry)`.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:149 SpawnStrategyRegistry.notifyUsedDynamic
    pub fn notify_used_dynamic(&self, action_context_registry: Option<&dyn ActionContextRegistry>) {
        for strategy in self.mnemonic_to_local_dynamic_strategies.values().flatten() {
            strategy.used_context(action_context_registry);
        }
        for strategy in self
            .mnemonic_to_remote_dynamic_strategies
            .values()
            .flatten()
        {
            strategy.used_context(action_context_registry);
        }
    }

    /// `getDynamicSpawnActionContexts(spawn, dynamicMode)`.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:159 SpawnStrategyRegistry.getDynamicSpawnActionContexts
    pub fn get_dynamic_spawn_action_contexts(
        &self,
        spawn: &dyn Spawn,
        dynamic_mode: DynamicMode,
    ) -> Vec<Arc<dyn SpawnStrategy>> {
        let mnemonic_to_dynamic_strategies = if dynamic_mode == DynamicMode::Remote {
            &self.mnemonic_to_remote_dynamic_strategies
        } else {
            &self.mnemonic_to_local_dynamic_strategies
        };
        if let Some(strategies) = mnemonic_to_dynamic_strategies.get(&spawn.get_mnemonic()) {
            return self
                .strategy_platform_filter
                .get_strategies(spawn, strategies.clone());
        }
        if let Some(strategies) = mnemonic_to_dynamic_strategies.get("") {
            return self
                .strategy_platform_filter
                .get_strategies(spawn, strategies.clone());
        }
        Vec::new()
    }

    /// `getRemoteLocalFallbackStrategy(spawn)`: an `AbstractSpawnStrategy` (see
    /// [`SpawnStrategy::as_abstract_spawn_strategy`]) or `None`.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:177 SpawnStrategyRegistry.getRemoteLocalFallbackStrategy
    pub fn get_remote_local_fallback_strategy(
        &self,
        spawn: &dyn Spawn,
    ) -> Option<Arc<dyn SpawnStrategy>> {
        // Java filters `Lists.newArrayList(remoteLocalFallbackStrategy)`, a list holding null when
        // there is no fallback strategy; a null element is never allowed by a platform filter and
        // is returned as null otherwise.
        StrategyPlatformFilter::check_platform_label(spawn);
        let strategy = self.remote_local_fallback_strategy.clone()?;
        self.strategy_platform_filter
            .get_strategies(spawn, vec![strategy])
            .into_iter()
            .next()
    }

    /// Notifies all (non-dynamic) strategies stored in this registry that they are used.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:191 SpawnStrategyRegistry.notifyUsed
    pub fn notify_used(&self, action_context_registry: Option<&dyn ActionContextRegistry>) {
        for (_, strategies) in &self.strategy_regex_filter.filter_to_strategies {
            for strategy in strategies {
                strategy.used_context(action_context_registry);
            }
        }
        for strategy in self.mnemonic_to_strategies.values().flatten() {
            strategy.used_context(action_context_registry);
        }
        for strategy in &self.default_strategies {
            strategy.used_context(action_context_registry);
        }
        if let Some(strategy) = &self.remote_local_fallback_strategy {
            strategy.used_context(action_context_registry);
        }
    }

    // PORT: skipped, tracing (logSpawnStrategies only writes the Java log).

    /// Returns a new builder suitable for creating instances of SpawnStrategyRegistry.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:265 SpawnStrategyRegistry.builder
    pub fn builder() -> SpawnStrategyRegistryBuilder {
        SpawnStrategyRegistryBuilder::new(
            SpawnStrategyPolicy::AllowAll,
            SpawnStrategyPolicy::AllowAll,
            SpawnStrategyPolicy::AllowAll,
        )
    }

    /// Returns a new builder suitable for creating instances of SpawnStrategyRegistry.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:273 SpawnStrategyRegistry.builder
    pub fn builder_with_policy(
        strategy_policy_proto: &StrategyPolicy,
    ) -> SpawnStrategyRegistryBuilder {
        SpawnStrategyRegistryBuilder::new(
            SpawnStrategyPolicy::create(
                &strategy_policy_proto
                    .mnemonic_policy
                    .clone()
                    .unwrap_or_default(),
            ),
            SpawnStrategyPolicy::create(
                &strategy_policy_proto
                    .dynamic_remote_policy
                    .clone()
                    .unwrap_or_default(),
            ),
            SpawnStrategyPolicy::create(
                &strategy_policy_proto
                    .dynamic_local_policy
                    .clone()
                    .unwrap_or_default(),
            ),
        )
    }
}

/// `SpawnStrategyRegistry.Builder`: collects the strategies and restrictions thereon for a
/// [`SpawnStrategyRegistry`].
///
/// To match a strategy to a spawn it needs to be both registered and its registered command-line
/// identifier has to match a filter on the spawn's progress message, a filter on the spawn's
/// mnemonic or be part of the default strategies.
///
/// Default strategies are either set explicitly or, if `set_default_strategies` is not called on
/// this builder, comprised of all registered strategies, in reverse registration order.
// Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:294 SpawnStrategyRegistry.Builder
pub struct SpawnStrategyRegistryBuilder {
    strategy_mapper: StrategyMapper,
    strategies_in_registration_order: Vec<String>,
    explicit_default_strategies: Vec<String>,
    strategy_policy: SpawnStrategyPolicy,
    dynamic_remote_policy: SpawnStrategyPolicy,
    dynamic_local_policy: SpawnStrategyPolicy,
    filter_and_identifiers: Vec<FilterAndIdentifiers>,
    mnemonic_to_identifiers: JavaStringHashMap<Vec<String>>,
    mnemonic_to_remote_dynamic_identifiers: JavaStringHashMap<Vec<String>>,
    mnemonic_to_local_dynamic_identifiers: JavaStringHashMap<Vec<String>>,
    // PORT: java-hash-order. Java uses a HashMap<Label, List<String>>; its iteration order only
    // decides which of several invalid --allowed_strategies_by_exec_platform entries is reported
    // first. Insertion order is used here.
    exec_platform_filters: IndexMap<Label, Vec<String>>,
    remote_local_fallback_strategy_identifier: Option<String>,
}

impl SpawnStrategyRegistryBuilder {
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:317 SpawnStrategyRegistry.Builder.<init>
    fn new(
        strategy_policy: SpawnStrategyPolicy,
        dynamic_remote_policy: SpawnStrategyPolicy,
        dynamic_local_policy: SpawnStrategyPolicy,
    ) -> SpawnStrategyRegistryBuilder {
        SpawnStrategyRegistryBuilder {
            strategy_mapper: StrategyMapper::new(),
            strategies_in_registration_order: Vec::new(),
            explicit_default_strategies: Vec::new(),
            strategy_policy,
            dynamic_remote_policy,
            dynamic_local_policy,
            filter_and_identifiers: Vec::new(),
            mnemonic_to_identifiers: JavaStringHashMap::new(),
            mnemonic_to_remote_dynamic_identifiers: JavaStringHashMap::new(),
            mnemonic_to_local_dynamic_identifiers: JavaStringHashMap::new(),
            exec_platform_filters: IndexMap::new(),
            remote_local_fallback_strategy_identifier: None,
        }
    }

    /// Adds a filter limiting any spawn whose owner's progress message matches the regular
    /// expression to only use strategies with the given command-line identifiers, in order.
    ///
    /// If multiple filters match the same spawn (including an identical filter) the order of last
    /// applicable filter registered by this method will be used.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:336 SpawnStrategyRegistry.Builder.addDescriptionFilter
    pub fn add_description_filter(
        &mut self,
        filter: RegexFilter,
        identifiers: &[String],
    ) -> &mut Self {
        self.filter_and_identifiers.push(FilterAndIdentifiers {
            filter,
            identifiers: identifiers.to_vec(),
        });
        self
    }

    /// `addExecPlatformFilter(execPlatform, identifiers)`.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:342 SpawnStrategyRegistry.Builder.addExecPlatformFilter
    pub fn add_exec_platform_filter(
        &mut self,
        exec_platform: Label,
        identifiers: &[String],
    ) -> &mut Self {
        self.exec_platform_filters
            .insert(exec_platform, identifiers.to_vec());
        self
    }

    /// Adds a filter limiting any spawn whose mnemonic (case-sensitively) matches the given
    /// mnemonic to only use strategies with the given command-line identifiers, in order.
    ///
    /// If the same mnemonic is registered multiple times the last such call will take precedence.
    /// Note that if a spawn matches a registered description filter that filter will take
    /// precedence over any mnemonic-based filters.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:359 SpawnStrategyRegistry.Builder.addMnemonicFilter
    pub fn add_mnemonic_filter(&mut self, mnemonic: &str, identifiers: &[String]) -> &mut Self {
        self.mnemonic_to_identifiers
            .put(mnemonic.to_string(), identifiers.to_vec());
        self
    }

    /// Sets the strategy names to use in the remote branch of dynamic execution for a set of
    /// action mnemonics (`Map.putAll`, in the given map's order).
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:373 SpawnStrategyRegistry.Builder.addDynamicRemoteStrategies
    pub fn add_dynamic_remote_strategies(
        &mut self,
        strategies: &IndexMap<String, Vec<String>>,
    ) -> &mut Self {
        for (k, v) in strategies {
            self.mnemonic_to_remote_dynamic_identifiers
                .put(k.clone(), v.clone());
        }
        self
    }

    /// Sets the strategy names to use in the local branch of dynamic execution for a number of
    /// action mnemonics (`Map.putAll`, in the given map's order).
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:387 SpawnStrategyRegistry.Builder.addDynamicLocalStrategies
    pub fn add_dynamic_local_strategies(
        &mut self,
        strategies: &IndexMap<String, Vec<String>>,
    ) -> &mut Self {
        for (k, v) in strategies {
            self.mnemonic_to_local_dynamic_identifiers
                .put(k.clone(), v.clone());
        }
        self
    }

    /// Registers a strategy implementation with this collector, distinguishing it from other
    /// strategies with the given command-line identifiers (of which at least one is required).
    ///
    /// If multiple strategies are registered with the same command-line identifier the last one so
    /// registered will take precedence.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:400 SpawnStrategyRegistry.Builder.registerStrategy
    pub fn register_strategy(
        &mut self,
        strategy: Arc<dyn SpawnStrategy>,
        command_line_identifiers: &[&str],
    ) -> &mut Self {
        blaze_util::check_argument!(
            !command_line_identifiers.is_empty(),
            "At least one commandLineIdentifier must be given"
        );
        for identifier in command_line_identifiers {
            self.strategy_mapper
                .register_strategy(identifier, strategy.clone());
            self.strategies_in_registration_order
                .push(identifier.to_string());
        }
        self
    }

    /// Explicitly sets the identifiers of default strategies to use if a spawn matches no
    /// filters.
    ///
    /// Note that if this method is not called on the builder, all registered strategies are
    /// considered default strategies, in reverse registration order.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:418 SpawnStrategyRegistry.Builder.setDefaultStrategies
    pub fn set_default_strategies(&mut self, default_strategies: &[String]) -> &mut Self {
        // Ensure there are actual strategies and the contents are not empty.
        blaze_util::check_argument!(!default_strategies.is_empty());
        blaze_util::check_argument!(default_strategies.iter().any(|s| !s.is_empty()));
        self.explicit_default_strategies = default_strategies.to_vec();
        self
    }

    /// Reset the default strategies (see `set_default_strategies`) to the reverse of the order
    /// they were registered in.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:432 SpawnStrategyRegistry.Builder.resetDefaultStrategies
    pub fn reset_default_strategies(&mut self) -> &mut Self {
        self.explicit_default_strategies = Vec::new();
        self
    }

    /// Sets the commandline identifier of the strategy to be used when falling back from remote
    /// to local execution. If provided it must match the commandline identifier of a registered
    /// strategy (at build time).
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:447 SpawnStrategyRegistry.Builder.setRemoteLocalFallbackStrategyIdentifier
    pub fn set_remote_local_fallback_strategy_identifier(
        &mut self,
        commandline_identifier: &str,
    ) -> &mut Self {
        self.remote_local_fallback_strategy_identifier = Some(commandline_identifier.to_string());
        self
    }

    /// `isStrategyRegistered(strategy)`.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:452 SpawnStrategyRegistry.Builder.isStrategyRegistered
    pub fn is_strategy_registered(&self, strategy: &str) -> bool {
        self.strategies_in_registration_order
            .iter()
            .any(|s| s == strategy)
    }

    /// Finalizes the construction of the registry. Fails if a strategy command-line identifier
    /// was used in a filter or the default strategies but no strategy for that identifier was
    /// registered.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:462 SpawnStrategyRegistry.Builder.build
    pub fn build(&self) -> Result<SpawnStrategyRegistry, AbruptExitException> {
        // LinkedListMultimap<RegexFilter, String> / <RegexFilter, SpawnStrategy>: keys in
        // insertion order, keys only present with at least one value.
        let mut filter_to_identifiers: Vec<(RegexFilter, Vec<String>)> = Vec::new();
        let mut filter_to_strategies: Vec<(RegexFilter, Vec<Arc<dyn SpawnStrategy>>)> = Vec::new();
        for filter_and_identifier in self.filter_and_identifiers.iter().rev() {
            let filter = &filter_and_identifier.filter;
            if !filter_to_identifiers.iter().any(|(f, _)| f == filter) {
                if !filter_and_identifier.identifiers.is_empty() {
                    filter_to_identifiers
                        .push((filter.clone(), filter_and_identifier.identifiers.clone()));
                }
                let strategies = self.strategy_mapper.to_strategies(
                    &filter_and_identifier.identifiers,
                    &format!("filter {filter}"),
                )?;
                if !strategies.is_empty() {
                    filter_to_strategies.push((filter.clone(), strategies));
                }
            }
        }

        // ImmutableSetMultimap<Label, SpawnStrategy>.
        let mut platform_to_strategies: IndexMap<Label, Vec<Arc<dyn SpawnStrategy>>> =
            IndexMap::new();
        for (platform, identifiers) in &self.exec_platform_filters {
            let strategies = self.strategy_mapper.to_strategies(
                identifiers,
                &format!("platform {}", platform.get_canonical_form()),
            )?;
            for strategy in strategies {
                let entry = platform_to_strategies.entry(platform.clone()).or_default();
                if !entry.iter().any(|s| same_strategy(s, &strategy)) {
                    entry.push(strategy);
                }
            }
        }

        let mut mnemonic_to_strategies: IndexMap<String, Vec<Arc<dyn SpawnStrategy>>> =
            IndexMap::new();
        for (mnemonic, identifiers) in self.mnemonic_to_identifiers.iter() {
            let sanitized_strategies = self
                .strategy_policy
                .apply_for_mnemonic(mnemonic, identifiers);
            let strategies = self
                .strategy_mapper
                .to_strategies(&sanitized_strategies, &format!("mnemonic {mnemonic}"))?;
            put_all(&mut mnemonic_to_strategies, mnemonic, strategies);
        }

        let mut mnemonic_to_local_strategies: IndexMap<String, Vec<Arc<dyn SpawnStrategy>>> =
            IndexMap::new();
        for (mnemonic, identifiers) in self.mnemonic_to_local_dynamic_identifiers.iter() {
            let sanitized_strategies = self
                .dynamic_local_policy
                .apply_for_mnemonic(mnemonic, identifiers);
            let strategies = self.strategy_mapper.to_sandboxed_strategies(
                &sanitized_strategies,
                &format!("local mnemonic {mnemonic}"),
            )?;
            put_all(&mut mnemonic_to_local_strategies, mnemonic, strategies);
        }

        let mut mnemonic_to_remote_strategies: IndexMap<String, Vec<Arc<dyn SpawnStrategy>>> =
            IndexMap::new();
        for (mnemonic, identifiers) in self.mnemonic_to_remote_dynamic_identifiers.iter() {
            let sanitized_strategies = self
                .dynamic_remote_policy
                .apply_for_mnemonic(mnemonic, identifiers);
            let strategies = self.strategy_mapper.to_sandboxed_strategies(
                &sanitized_strategies,
                &format!("remote mnemonic {mnemonic}"),
            )?;
            put_all(&mut mnemonic_to_remote_strategies, mnemonic, strategies);
        }

        let mut remote_local_fallback_strategy = None;
        if let Some(identifier) = &self.remote_local_fallback_strategy_identifier {
            let strategy = self
                .strategy_mapper
                .to_strategy(identifier, "remote fallback strategy")?;
            if strategy.as_abstract_spawn_strategy().is_none() {
                // TODO(schmitt): Check if all strategies can use the same base and remove check if
                // so.
                return Err(create_exit_exception(
                    format!(
                        "'{}' was requested for the remote fallback strategy but is not an abstract spawn strategy (which is required for remote fallback execution).",
                        strategy.get_simple_class_name()
                    ),
                    ExecutionOptionsCode::RemoteFallbackStrategyNotAbstractSpawn,
                ));
            }
            remote_local_fallback_strategy = Some(strategy);
        }

        let default_strategies = if self.explicit_default_strategies.is_empty() {
            // Use the strategies as registered, in reverse order.
            let reversed: Vec<String> = self
                .strategies_in_registration_order
                .iter()
                .rev()
                .cloned()
                .collect();
            self.strategy_mapper.to_strategies(
                &self.strategy_policy.apply(&reversed),
                "implicit default strategies",
            )?
        } else {
            self.strategy_mapper.to_strategies(
                &self
                    .strategy_policy
                    .apply(&self.explicit_default_strategies),
                "explicit default strategies",
            )?
        };

        Ok(SpawnStrategyRegistry::new(
            mnemonic_to_strategies,
            StrategyRegexFilter {
                strategy_mapper: self.strategy_mapper.clone(),
                strategy_policy: self.strategy_policy.clone(),
                filter_to_identifiers,
                filter_to_strategies,
            },
            StrategyPlatformFilter {
                platform_to_strategies,
            },
            default_strategies,
            mnemonic_to_remote_strategies,
            mnemonic_to_local_strategies,
            remote_local_fallback_strategy,
        ))
    }

    /// `toStrategy(identifier, requestName)`.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:565 SpawnStrategyRegistry.Builder.toStrategy
    pub fn to_strategy(
        &self,
        identifier: &str,
        request_name: &str,
    ) -> Result<Arc<dyn SpawnStrategy>, AbruptExitException> {
        self.strategy_mapper.to_strategy(identifier, request_name)
    }
}

/// `ImmutableListMultimap.Builder.putAll(key, values)`: a key without values is not added.
fn put_all(
    map: &mut IndexMap<String, Vec<Arc<dyn SpawnStrategy>>>,
    key: &str,
    values: Vec<Arc<dyn SpawnStrategy>>,
) {
    if values.is_empty() {
        return;
    }
    map.entry(key.to_string()).or_default().extend(values);
}

/// `Builder.FilterAndIdentifiers`.
// Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:762 SpawnStrategyRegistry.FilterAndIdentifiers
#[derive(Clone)]
struct FilterAndIdentifiers {
    filter: RegexFilter,
    identifiers: Vec<String>,
}

/// Filter that applies strategy_regexp while respecting the command's strategy-policy.
// Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:572 SpawnStrategyRegistry.StrategyRegexFilter
struct StrategyRegexFilter {
    // PORT: Java shares the builder's mapper; strategies registered on the builder after build()
    // are not visible here.
    strategy_mapper: StrategyMapper,
    strategy_policy: SpawnStrategyPolicy,
    filter_to_identifiers: Vec<(RegexFilter, Vec<String>)>,
    filter_to_strategies: Vec<(RegexFilter, Vec<Arc<dyn SpawnStrategy>>)>,
}

impl StrategyRegexFilter {
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:589 SpawnStrategyRegistry.StrategyRegexFilter.getStrategies
    fn get_strategies(
        &self,
        mnemonic: &str,
        description: &str,
        reporter: Option<&dyn EventHandler>,
    ) -> Vec<Arc<dyn SpawnStrategy>> {
        for (filter, identifiers) in &self.filter_to_identifiers {
            if filter.is_included(description) {
                if let Some(reporter) = reporter {
                    // TODO(schmitt): Why is this done here and not after running canExec?
                    reporter.handle(&Event::progress(format!(
                        "{} with context {}",
                        description,
                        java_list_to_string(identifiers)
                    )));
                }
                // Apply the policy to the identifiers.
                let sanitized_strategies = self
                    .strategy_policy
                    .apply_for_mnemonic(mnemonic, identifiers);
                match self
                    .strategy_mapper
                    .to_strategies(&sanitized_strategies, &format!("filter {filter}"))
                {
                    Ok(strategies) => {
                        if strategies.is_empty() {
                            // If after sanitizing we get the empty list of strategies, we should
                            // return null to indicate that default strategies should be used.
                            return Vec::new();
                        }
                        return strategies;
                    }
                    Err(e) => {
                        // We should not reach this code because the mapping to strategies already
                        // applied while building filterToStrategies
                        panic!(
                            "IllegalStateException: Failed to apply policy for to strategies that were already applied for mnemonic {mnemonic} and filter {filter}: {e}"
                        );
                    }
                }
            }
        }

        // Return the empty list if no filter matches.
        Vec::new()
    }
}

// Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:639 SpawnStrategyRegistry.StrategyPlatformFilter
struct StrategyPlatformFilter {
    platform_to_strategies: IndexMap<Label, Vec<Arc<dyn SpawnStrategy>>>,
}

impl StrategyPlatformFilter {
    fn check_platform_label(spawn: &dyn Spawn) -> Label {
        spawn.get_execution_platform_label().expect(
            "NullPointerException: Attempting to spawn action without an execution platform.",
        )
    }

    /// Gets strategies for the given spawn that are allowed by the execution platform: a subset of
    /// `candidate_strategies` (order preserved), or all if no restrictions are in place.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:660 SpawnStrategyRegistry.StrategyPlatformFilter.getStrategies
    fn get_strategies(
        &self,
        spawn: &dyn Spawn,
        candidate_strategies: Vec<Arc<dyn SpawnStrategy>>,
    ) -> Vec<Arc<dyn SpawnStrategy>> {
        let platform_label = StrategyPlatformFilter::check_platform_label(spawn);

        if let Some(allowed_strategies) = self.platform_to_strategies.get(&platform_label) {
            let mut filtered_strategies = Vec::new();
            for strategy in candidate_strategies {
                if allowed_strategies
                    .iter()
                    .any(|s| same_strategy(s, &strategy))
                {
                    filtered_strategies.push(strategy);
                }
            }
            return filtered_strategies;
        }

        candidate_strategies
    }
}

/// Maps the strategy identifier (e.g. "local", "worker"..) to the real strategy.
// Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:696 SpawnStrategyRegistry.StrategyMapper
#[derive(Clone)]
struct StrategyMapper {
    identifier_to_strategy: JavaStringHashMap<Arc<dyn SpawnStrategy>>,
}

impl StrategyMapper {
    fn new() -> StrategyMapper {
        StrategyMapper {
            identifier_to_strategy: JavaStringHashMap::new(),
        }
    }

    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:702 SpawnStrategyRegistry.StrategyMapper.registerStrategy
    fn register_strategy(&mut self, identifier: &str, strategy: Arc<dyn SpawnStrategy>) {
        self.identifier_to_strategy
            .put(identifier.to_string(), strategy);
    }

    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:706 SpawnStrategyRegistry.StrategyMapper.toStrategies
    fn to_strategies(
        &self,
        identifiers: &[String],
        request_name: &str,
    ) -> Result<Vec<Arc<dyn SpawnStrategy>>, AbruptExitException> {
        let mut strategies = Vec::new();
        for identifier in identifiers {
            if identifier.is_empty() {
                continue;
            }
            strategies.push(self.to_strategy(identifier, request_name)?);
        }
        Ok(strategies)
    }

    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:718 SpawnStrategyRegistry.StrategyMapper.toStrategy
    fn to_strategy(
        &self,
        identifier: &str,
        request_name: &str,
    ) -> Result<Arc<dyn SpawnStrategy>, AbruptExitException> {
        match self.identifier_to_strategy.get(identifier) {
            Some(strategy) => Ok(strategy.clone()),
            None => {
                let keys: Vec<&str> = self
                    .identifier_to_strategy
                    .iter()
                    .map(|(k, _)| k.as_str())
                    .collect();
                Err(create_exit_exception(
                    format!(
                        "'{}' was requested for {} but no strategy with that identifier was registered. Valid values are: [{}]",
                        identifier,
                        request_name,
                        keys.join(", ")
                    ),
                    ExecutionOptionsCode::StrategyNotFound,
                ))
            }
        }
    }

    // Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:731 SpawnStrategyRegistry.StrategyMapper.toSandboxedStrategies
    fn to_sandboxed_strategies(
        &self,
        identifiers: &[String],
        request_name: &str,
    ) -> Result<Vec<Arc<dyn SpawnStrategy>>, AbruptExitException> {
        let strategies = self.to_strategies(identifiers, request_name)?;
        for strategy in &strategies {
            if strategy.as_sandboxed_spawn_strategy().is_none() {
                return Err(create_exit_exception(
                    format!(
                        "'{}' was requested for {} but is not a sandboxed strategy (which is required for dynamic execution).",
                        strategy.get_simple_class_name(),
                        request_name
                    ),
                    ExecutionOptionsCode::DynamicStrategyNotSandboxed,
                ));
            }
        }
        Ok(strategies)
    }
}

// Java: com/google/devtools/build/lib/exec/SpawnStrategyRegistry.java:752 SpawnStrategyRegistry.createExitException
fn create_exit_exception(
    message: String,
    detailed_code: ExecutionOptionsCode,
) -> AbruptExitException {
    AbruptExitException::new(FailureDetail {
        message,
        category: Some(failure_detail::Category::ExecutionOptions(
            failure_details::ExecutionOptions {
                code: detailed_code as i32,
            },
        )),
    })
}

/// Java `List<String>.toString()`.
fn java_list_to_string(items: &[String]) -> String {
    format!("[{}]", items.join(", "))
}

/// Java `String.hashCode()` over UTF-16 code units.
// Java: java/lang/String.java String.hashCode
fn java_string_hash_code(s: &str) -> i32 {
    let mut h: i32 = 0;
    for unit in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(unit as i32);
    }
    h
}

/// A `java.util.HashMap<String, V>` written only with `put`, simulated so that its iteration
/// order (which reaches error messages such as "Valid values are: [...]") matches Java's:
///
/// - the table is allocated lazily with 16 buckets, threshold 12;
/// - a new key is appended at the tail of its bucket; an existing key keeps its position;
/// - after appending to a bucket that already had 8 entries, a table with fewer than 64 buckets
///   is resized (`treeifyBin`);
/// - after an insertion, the table doubles when `size > threshold`; a resize splits each bucket
///   into a "lo" and a "hi" list keeping relative order.
///
/// PORT: java-hash-order. Treeified bins (9+ colliding keys in a table of 64+ buckets) are kept
/// as plain lists; Java's red-black bins could iterate differently. Strategy identifiers and
/// mnemonics never get there.
#[derive(Clone)]
struct JavaStringHashMap<V> {
    /// Each bucket in iteration order: (spread hash, key, value).
    table: Vec<Vec<(u32, String, V)>>,
    size: usize,
    threshold: usize,
}

impl<V> JavaStringHashMap<V> {
    const DEFAULT_INITIAL_CAPACITY: usize = 16;
    const TREEIFY_THRESHOLD: usize = 8;
    const MIN_TREEIFY_CAPACITY: usize = 64;

    fn new() -> Self {
        JavaStringHashMap {
            table: Vec::new(),
            size: 0,
            threshold: 0,
        }
    }

    /// `HashMap.hash(key)`.
    fn spread(key: &str) -> u32 {
        let h = java_string_hash_code(key) as u32;
        h ^ (h >> 16)
    }

    // Java: java/util/HashMap.java HashMap.resize
    fn resize(&mut self) {
        let old_cap = self.table.len();
        if old_cap == 0 {
            self.table = (0..Self::DEFAULT_INITIAL_CAPACITY)
                .map(|_| Vec::new())
                .collect();
            self.threshold = Self::DEFAULT_INITIAL_CAPACITY * 3 / 4;
            return;
        }
        let new_cap = old_cap * 2;
        let mut new_table: Vec<Vec<(u32, String, V)>> = (0..new_cap).map(|_| Vec::new()).collect();
        for (j, bucket) in std::mem::take(&mut self.table).into_iter().enumerate() {
            for entry in bucket {
                if entry.0 as usize & old_cap == 0 {
                    new_table[j].push(entry);
                } else {
                    new_table[j + old_cap].push(entry);
                }
            }
        }
        self.table = new_table;
        self.threshold *= 2;
    }

    // Java: java/util/HashMap.java HashMap.putVal
    fn put(&mut self, key: String, value: V) -> Option<V> {
        if self.table.is_empty() {
            self.resize();
        }
        let hash = Self::spread(&key);
        let n = self.table.len();
        let i = (n - 1) & hash as usize;
        let bucket = &mut self.table[i];
        if let Some(e) = bucket.iter_mut().find(|e| e.0 == hash && e.1 == key) {
            return Some(std::mem::replace(&mut e.2, value));
        }
        let len_before = bucket.len();
        bucket.push((hash, key, value));
        if len_before >= Self::TREEIFY_THRESHOLD && self.table.len() < Self::MIN_TREEIFY_CAPACITY {
            self.resize();
        }
        self.size += 1;
        if self.size > self.threshold {
            self.resize();
        }
        None
    }

    fn get(&self, key: &str) -> Option<&V> {
        if self.table.is_empty() {
            return None;
        }
        let hash = Self::spread(key);
        let i = (self.table.len() - 1) & hash as usize;
        self.table[i]
            .iter()
            .find(|e| e.0 == hash && e.1 == key)
            .map(|e| &e.2)
    }

    fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.table.iter().flatten().map(|e| (&e.1, &e.2))
    }
}
