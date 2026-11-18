//! Port of `query2/PostAnalysisQueryEnvironment.java` (with the generic parts of
//! `query2/common/AbstractBlazeQueryEnvironment.java` it inherits), `query2/SkyQueryUtils.java`,
//! `query2/NamedThreadSafeOutputFormatterCallback.java`,
//! `skyframe/GraphBackedRecursivePackageProvider.java` and
//! `skyframe/RecursivePkgValueRootPackageExtractor.java`.
//!
//! PORT: Java's abstract class `PostAnalysisQueryEnvironment<T>` with the subclasses
//! `ConfiguredTargetQueryEnvironment` (`T = CqueryNode`) and `ActionGraphQueryEnvironment`
//! (`T = ConfiguredTargetValue`) is one generic struct; the per-node-type methods are
//! [`PostAnalysisNode`] (both subclasses implement them by unwrapping the configured target) and
//! the `TargetAccessor<T>` implementations in `cquery` and `aquery`. blaze_query's
//! `AbstractBlazeQueryEnvironment` is specialized to `Target`; its generic parts used here
//! (`evaluateQueryInternal`, `eval`, `handleError`, `transitiveLoadFiles`) are ported generically
//! in this file.
//!
//! PORT: java-hash-order. Java's result sets (`ThreadSafeMutableKeyExtractorBackedSetImpl` over a
//! `ConcurrentHashMap<ActionLookupKey, T>`) iterate in hash order of the keys; here they iterate
//! in insertion order. cquery/aquery outputs are compared as sets.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use blaze_actions::ActionLookupKeyRef;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_key::{CONFIGURED_TARGET, ConfiguredTargetKey};
use blaze_analysis::configured_target_value::ConfiguredTargetValue;
use blaze_analysis::configuredtargets::RuleConfiguredTarget;
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::{
    Label, PackageIdentifier, RepositoryName, TargetParsingException, TargetPatternParser,
};
use blaze_events::{Event, EventHandler, ExtendedEventHandler};
use blaze_packages::exceptions::NoSuchPackageException;
use blaze_packages::package::Package;
use blaze_packages::target::{LabelPrinter, Target};
use blaze_protos::failure_details::{
    FailureDetail, Query, configurable_query, failure_detail::Category, query,
};
use blaze_query::common::abstract_blaze_query_environment::{
    EvaluateQueryError, QueryErrorSensingEventHandler,
};
use blaze_query::compat::failure_details;
use blaze_query_engine::callback::{
    IoException, IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::query_util::{self, MinDepthUniquifierImpl, UniquifierImpl};
use blaze_query_engine::{
    Callback, DetailedExitCode, EvaluateExpression, InterruptedException, KeyExtractor,
    MinDepthUniquifier, OutputFormatterCallback, QueryEnvironment, QueryEnvironmentFutures,
    QueryError, QueryEvalResult, QueryException, QueryExpression, QueryExpressionContext,
    QueryFunction, QueryTaskFuture, Setting, TargetAccessor, TargetNotFoundException,
    ThreadSafeMutableSet, ThreadSafeOutputFormatterCallback, TransitiveLoadFilesHelper, Uniquifier,
    default_query_functions,
};
use blaze_sky_loading::ignored_subdirectories::{IgnoredSubdirectories, IgnoredSubdirectoriesKey};
use blaze_sky_loading::package_function::{PackageKey, PackageValue};
use blaze_sky_loading::package_lookup::{PackageLookupKey, PackageLookupValue};
use blaze_sky_loading::pkgcache::PathPackageLocator;
use blaze_sky_loading::recursive_pkg::{RecursivePkgKey, RecursivePkgValue};
use blaze_sky_loading::repos::{RepositoryDirectoryKey, RepositoryDirectoryValue};
use blaze_sky_loading::target_pattern_eval::{self, TargetPatternEvalError};
use blaze_sky_loading::target_pattern_function::{
    FilteringPolicy, ProviderError, RecursivePackageProvider,
    RecursivePackageProviderBackedTargetPatternResolver,
};
use blaze_skyframe::{SkyKey, WalkableGraph};
use blaze_vfs::{PathFragment, Root, RootedPath};
use indexmap::{IndexMap, IndexSet};

/// `SkyQueryEnvironment.DEFAULT_THREAD_COUNT`.
const DEFAULT_THREAD_COUNT: i32 = 200;

// -------------------------------------------------------------------------------------------
// Node types.

/// The node types of post-analysis queries: `CqueryNode` (a `ConfiguredTarget`) for cquery and
/// `ConfiguredTargetValue` for aquery.
pub trait PostAnalysisNode: Clone + 'static {
    /// The configured target of the node.
    fn get_configured_target(&self) -> &ConfiguredTarget;

    /// Java `getValueFromKey`: the node for a `CONFIGURED_TARGET` value.
    fn from_configured_target_value(value: Arc<ConfiguredTargetValue>) -> Self;
}

impl PostAnalysisNode for ConfiguredTarget {
    fn get_configured_target(&self) -> &ConfiguredTarget {
        self
    }

    fn from_configured_target_value(value: Arc<ConfiguredTargetValue>) -> Self {
        value.get_configured_target().clone()
    }
}

impl PostAnalysisNode for Arc<ConfiguredTargetValue> {
    fn get_configured_target(&self) -> &ConfiguredTarget {
        ConfiguredTargetValue::get_configured_target(self)
    }

    fn from_configured_target_value(value: Arc<ConfiguredTargetValue>) -> Self {
        value
    }
}

/// The `ActionLookupKey` of a node (Java `getConfiguredTargetKey(T)`; both subclasses return the
/// configured target's lookup key).
pub fn get_configured_target_key<T: PostAnalysisNode>(target: &T) -> ActionLookupKeyRef {
    target.get_configured_target().get_lookup_key().clone()
}

/// The `ConfiguredTargetKey` behind a lookup key (Java `ConfiguredTargetKey.fromConfiguredTarget`).
pub fn as_configured_target_key(key: &ActionLookupKeyRef) -> Option<&ConfiguredTargetKey> {
    key.0.as_any().downcast_ref::<ConfiguredTargetKey>()
}

/// The SkyKey of a lookup key.
pub fn lookup_key_to_sky_key(key: &ActionLookupKeyRef) -> SkyKey {
    match as_configured_target_key(key) {
        Some(k) => SkyKey::new(k.clone()),
        None => blaze_util::unported!(
            "PostAnalysisQueryEnvironment.getConfiguredTargetKey",
            "aspect keys as query nodes ({})",
            key
        ),
    }
}

/// A `KeyExtractor` over [`get_configured_target_key`].
pub fn configured_target_key_extractor<T: PostAnalysisNode>()
-> Arc<dyn KeyExtractor<T, ActionLookupKeyRef>> {
    Arc::new(|t: &T| get_configured_target_key(t))
}

/// `ThreadSafeMutableKeyExtractorBackedSetImpl` keyed by the configured target key.
///
/// PORT: java-hash-order (see the module documentation): insertion order.
pub struct KeyedNodeSet<T> {
    map: Mutex<IndexMap<ActionLookupKeyRef, T>>,
}

impl<T> KeyedNodeSet<T> {
    pub fn new() -> Self {
        KeyedNodeSet {
            map: Mutex::new(IndexMap::new()),
        }
    }
}

impl<T: PostAnalysisNode> ThreadSafeMutableSet<T> for KeyedNodeSet<T> {
    fn add(&self, element: T) -> bool {
        let key = get_configured_target_key(&element);
        let mut map = self.map.lock().unwrap();
        if map.contains_key(&key) {
            return false;
        }
        map.insert(key, element);
        true
    }

    fn contains(&self, element: &T) -> bool {
        self.map
            .lock()
            .unwrap()
            .contains_key(&get_configured_target_key(element))
    }

    fn remove(&self, element: &T) -> bool {
        self.map
            .lock()
            .unwrap()
            .shift_remove(&get_configured_target_key(element))
            .is_some()
    }

    fn size(&self) -> i32 {
        self.map.lock().unwrap().len() as i32
    }

    fn to_vec(&self) -> Vec<T> {
        self.map.lock().unwrap().values().cloned().collect()
    }
}

// -------------------------------------------------------------------------------------------
// TopLevelConfigurations.

/// A wrapper class for the set of top-level configurations in a query.
// Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:738 PostAnalysisQueryEnvironment.TopLevelConfigurations
pub struct TopLevelConfigurations {
    /// A map of non-null configured top-level targets sorted by configuration checksum.
    non_nulls: IndexMap<Label, Arc<BuildConfigurationValue>>,
    /// `nonNulls` may often have many duplicate values in its value set so we store a sorted set
    /// of all the non-null configurations here.
    non_null_configs: Vec<Arc<BuildConfigurationValue>>,
    /// A list of null configured top-level targets.
    nulls: Vec<Label>,
}

impl TopLevelConfigurations {
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:752 TopLevelConfigurations.<init>
    pub fn new(
        top_level_targets_and_configurations: &[blaze_sky_top::skyframe_executor::TargetAndConfiguration],
    ) -> Self {
        let mut non_nulls: IndexMap<Label, Arc<BuildConfigurationValue>> = IndexMap::new();
        let mut nulls: Vec<Label> = Vec::new();
        for target_and_configuration in top_level_targets_and_configurations {
            match target_and_configuration.get_configuration() {
                None => nulls.push(target_and_configuration.get_label().clone()),
                Some(configuration) => {
                    let label = target_and_configuration.get_label().clone();
                    // ImmutableMap.Builder.buildOrThrow
                    if non_nulls
                        .insert(label.clone(), configuration.clone())
                        .is_some()
                    {
                        panic!("IllegalArgumentException: Multiple entries with same key: {label}");
                    }
                }
            }
        }
        // ImmutableSortedSet.copyOf(Comparator.comparing(checksum), nonNulls.values())
        let mut non_null_configs: Vec<Arc<BuildConfigurationValue>> = Vec::new();
        for config in non_nulls.values() {
            if !non_null_configs
                .iter()
                .any(|c| c.checksum() == config.checksum())
            {
                non_null_configs.push(config.clone());
            }
        }
        non_null_configs.sort_by(|a, b| a.checksum().cmp(b.checksum()));
        TopLevelConfigurations {
            non_nulls,
            non_null_configs,
            nulls,
        }
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:772 TopLevelConfigurations.isTopLevelTarget
    pub fn is_top_level_target(&self, label: &Label) -> bool {
        self.non_nulls.contains_key(label) || self.nulls.contains(label)
    }

    /// This method returns the configuration of a top-level target if it's not null-configured
    /// and otherwise returns `None` (signifying it is null configured).
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:779 TopLevelConfigurations.getConfigurationForTopLevelTarget
    pub fn get_configuration_for_top_level_target(
        &self,
        label: &Label,
    ) -> Option<&Arc<BuildConfigurationValue>> {
        blaze_util::check_argument!(
            self.is_top_level_target(label),
            "Attempting to get top-level configuration for non-top-level target {}.",
            label
        );
        self.non_nulls.get(label)
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:787 TopLevelConfigurations.getConfigurations
    pub fn get_configurations(&self) -> Vec<Option<Arc<BuildConfigurationValue>>> {
        let mut result: Vec<Option<Arc<BuildConfigurationValue>>> =
            self.non_null_configs.iter().cloned().map(Some).collect();
        if !self.nulls.is_empty() {
            result.push(None);
        }
        result
    }
}

// -------------------------------------------------------------------------------------------
// NamedThreadSafeOutputFormatterCallback.

/// A `ThreadSafeOutputFormatterCallback` with a name (the `--output` value it implements).
// Java: com/google/devtools/build/lib/query2/NamedThreadSafeOutputFormatterCallback.java:25 NamedThreadSafeOutputFormatterCallback
pub trait NamedThreadSafeOutputFormatterCallback<T>: ThreadSafeOutputFormatterCallback<T> {
    fn get_name(&self) -> String;
}

// Java: com/google/devtools/build/lib/query2/NamedThreadSafeOutputFormatterCallback.java:30 NamedThreadSafeOutputFormatterCallback.callbackNames
pub fn callback_names<T>(
    callbacks: &[Box<dyn NamedThreadSafeOutputFormatterCallback<T> + '_>],
) -> String {
    callbacks
        .iter()
        .map(|c| c.get_name())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Returns the index of the callback named `type_`, if any.
// Java: com/google/devtools/build/lib/query2/NamedThreadSafeOutputFormatterCallback.java:38 NamedThreadSafeOutputFormatterCallback.selectCallback
pub fn select_callback<T>(
    type_: &str,
    callbacks: &[Box<dyn NamedThreadSafeOutputFormatterCallback<T> + '_>],
) -> Option<usize> {
    callbacks.iter().position(|c| c.get_name() == type_)
}

// -------------------------------------------------------------------------------------------
// The environment.

/// A dependency with whether it is attached implicitly.
// Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:650 PostAnalysisQueryEnvironment.ClassifiedDependency
#[derive(Clone)]
struct ClassifiedDependency<T> {
    implicit: bool,
    dependency: T,
}

/// `QueryEnvironment` that runs queries based on results from the analysis phase.
///
/// This environment can theoretically be used for multiple queries, but currently is only ever
/// used for one over the course of its lifetime.
// Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:112 PostAnalysisQueryEnvironment
pub struct PostAnalysisQueryEnvironment<T> {
    // AbstractBlazeQueryEnvironment
    event_handler: Arc<QueryErrorSensingEventHandler>,
    event_handler_dyn: Arc<dyn ExtendedEventHandler>,
    keep_going: bool,
    /// Java `dependencyFilter == DependencyFilter.ALL_DEPS` (an identity comparison: the filter
    /// is ALL_DEPS exactly when no filtering setting is enabled).
    dependency_filter_is_all_deps: bool,
    pub(crate) settings: HashSet<Setting>,
    extra_functions: Vec<Arc<dyn QueryFunction<T>>>,
    label_printer: LabelPrinter,
    // PostAnalysisQueryEnvironment
    pub(crate) top_level_configurations: Arc<TopLevelConfigurations>,
    main_repo_target_parser: TargetPatternParser,
    pkg_path: PathPackageLocator,
    pub(crate) graph: Arc<dyn WalkableGraph>,
    /// Stores every configuration in the transitive closure of the build graph as a map from its
    /// user-friendly hash to the configuration itself.
    pub(crate) transitive_configurations: Arc<IndexMap<String, Arc<BuildConfigurationValue>>>,
}

impl<T: PostAnalysisNode> PostAnalysisQueryEnvironment<T> {
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:141 PostAnalysisQueryEnvironment.<init>
    pub fn new(
        keep_going: bool,
        event_handler: Arc<dyn ExtendedEventHandler>,
        extra_functions: Vec<Arc<dyn QueryFunction<T>>>,
        top_level_configurations: TopLevelConfigurations,
        transitive_configurations: IndexMap<String, Arc<BuildConfigurationValue>>,
        main_repo_target_parser: TargetPatternParser,
        pkg_path: PathPackageLocator,
        graph: Arc<dyn WalkableGraph>,
        settings: HashSet<Setting>,
        label_printer: LabelPrinter,
    ) -> Self {
        // super(keepGoing, true, Rule.ALL_LABELS, eventHandler, settings, extraFunctions,
        // labelPrinter)
        let event_handler = Arc::new(QueryErrorSensingEventHandler::new(event_handler));
        PostAnalysisQueryEnvironment {
            event_handler_dyn: event_handler.clone(),
            event_handler,
            keep_going,
            dependency_filter_is_all_deps: is_all_deps_filter(&settings),
            settings,
            extra_functions,
            label_printer,
            top_level_configurations: Arc::new(top_level_configurations),
            main_repo_target_parser,
            pkg_path,
            graph,
            transitive_configurations: Arc::new(transitive_configurations),
        }
    }

    /// `eventHandler` as the `ExtendedEventHandler` loading calls take.
    pub fn event_handler(&self) -> &Arc<dyn ExtendedEventHandler> {
        &self.event_handler_dyn
    }

    pub fn get_label_printer(&self) -> &LabelPrinter {
        &self.label_printer
    }

    pub fn get_graph(&self) -> &Arc<dyn WalkableGraph> {
        &self.graph
    }

    pub fn get_transitive_configurations(&self) -> &IndexMap<String, Arc<BuildConfigurationValue>> {
        &self.transitive_configurations
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:176 PostAnalysisQueryEnvironment.evaluateQuery
    pub fn evaluate_query(
        &self,
        expr: &QueryExpression,
        callback: &dyn ThreadSafeOutputFormatterCallback<T>,
    ) -> Result<QueryEvalResult, EvaluateQueryError>
    where
        Self: TargetAccessor<T>,
    {
        self.before_evaluate_query()?;
        self.evaluate_query_internal(expr, callback)
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:183 PostAnalysisQueryEnvironment.beforeEvaluateQuery
    fn before_evaluate_query(&self) -> Result<(), QueryException> {
        // PORT: the graph is given at construction; the resolver (GraphBackedRecursivePackageProvider
        // over it) is created per pattern in get_targets_matching_pattern.
        check_settings(&self.settings)
    }

    // TODO(bazel-team): It's weird that this untemplated function exists. Fix? Or don't implement?
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:219 PostAnalysisQueryEnvironment.getTarget
    pub fn get_target(&self, label: &Label) -> Result<Target, TargetNotFoundException> {
        let value = self
            .graph
            .get_value(&SkyKey::new(PackageKey::key(
                label.get_package_identifier().clone(),
            )))
            .unwrap_or_else(|| {
                panic!(
                    "NullPointerException: package {} not in the graph",
                    label.get_package_identifier()
                )
            })
            .downcast::<PackageValue>()
            .unwrap_or_else(|_| panic!("PACKAGE value"));
        match value.get_package().get_target(label.get_name()) {
            Ok(t) => Ok(t.clone()),
            Err(e) => {
                let detailed_exit_code = e.get_detailed_exit_code();
                Err(TargetNotFoundException::new(
                    Arc::new(e.clone()),
                    failure_details::from_packages(&detailed_exit_code),
                ))
            }
        }
    }

    /// The target behind a node; panics like Java's accessors (`IllegalStateException`).
    pub fn get_target_or_panic(&self, label: &Label) -> Target {
        match self.get_target(label) {
            Ok(t) => t,
            Err(e) => panic!(
                "IllegalStateException: Unable to get target from package in accessor.: {}",
                e.get_message()
            ),
        }
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:247 PostAnalysisQueryEnvironment.getConfiguredTargetValue
    pub fn get_configured_target_value(&self, key: &SkyKey) -> Option<Arc<ConfiguredTargetValue>> {
        blaze_sky_top::skyframe_executor::get_configured_target_value(&*self.graph, key)
    }

    /// Java `getValueFromKey` (both subclasses: the value of a `CONFIGURED_TARGET` key).
    pub fn get_value_from_key(&self, key: &SkyKey) -> Option<T> {
        self.graph_lookup().get_value_from_key(key)
    }

    /// The graph, top-level and transitive configurations (what `ConfigFunction` reads).
    pub fn graph_lookup(&self) -> PostAnalysisGraph {
        PostAnalysisGraph {
            graph: self.graph.clone(),
            top_level_configurations: self.top_level_configurations.clone(),
            transitive_configurations: self.transitive_configurations.clone(),
        }
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:263 PostAnalysisQueryEnvironment.getIgnoredSubdirectories
    fn get_ignored_subdirectories(
        &self,
        repository_name: &RepositoryName,
    ) -> IgnoredSubdirectories {
        match self
            .graph
            .get_value(&SkyKey::new(IgnoredSubdirectoriesKey::key(
                repository_name.clone(),
            ))) {
            None => IgnoredSubdirectories::empty(),
            Some(v) => v
                .downcast::<blaze_sky_loading::ignored_subdirectories::IgnoredSubdirectoriesValue>()
                .unwrap_or_else(|_| panic!("IGNORED_SUBDIRECTORIES value"))
                .as_ignored_subdirectories()
                .clone(),
        }
    }

    /// The configuration of a node.
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:537 ConfiguredTargetQueryEnvironment.getConfiguration
    pub fn get_configuration(&self, target: &T) -> Option<Arc<BuildConfigurationValue>> {
        let key = target.get_configured_target().get_configuration_key()?;
        blaze_sky_top::skyframe_executor::get_configuration_from_graph(&*self.graph, Some(&key))
    }

    /// This method has to exist because `AliasConfiguredTarget.getLabel()` returns the label of
    /// the "actual" target instead of the alias target.
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:477 ConfiguredTargetQueryEnvironment.getCorrectLabel
    pub fn get_correct_label(&self, target: &T) -> Label {
        // Dereference any aliases that might be present.
        target.get_configured_target().get_original_label()
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:325 ConfiguredTargetQueryEnvironment.getConfiguredTarget
    pub fn get_configured_target(
        &self,
        label: &Label,
        configuration: Option<&Arc<BuildConfigurationValue>>,
    ) -> Option<T> {
        self.graph_lookup()
            .get_configured_target(label, configuration)
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:483 ConfiguredTargetQueryEnvironment.getTargetConfiguredTarget
    pub fn get_target_configured_target(&self, label: &Label) -> Option<T> {
        self.graph_lookup().get_target_configured_target(label)
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:501 ConfiguredTargetQueryEnvironment.getNullConfiguredTarget
    pub fn get_null_configured_target(&self, label: &Label) -> Option<T> {
        self.graph_lookup().get_null_configured_target(label)
    }

    /// Returns all configured targets in Skyframe with the given label.
    ///
    /// If there are no matches, returns an empty list.
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:363 ConfiguredTargetQueryEnvironment.getConfiguredTargetsForLabel
    fn get_configured_targets_for_label(&self, label: &Label) -> Vec<T> {
        let mut ans: Vec<T> = Vec::new();
        let mut extra_configured_target_keys: Option<HashSet<ConfiguredTargetKey>> = None;
        for configuration_value in self.transitive_configurations.values() {
            let configuration_key = configuration_value.get_key();
            let Some(target) = self.get_value_from_key(&SkyKey::new(
                ConfiguredTargetKey::builder()
                    .set_label(label.clone())
                    .set_configuration_key(Some(configuration_key.clone()))
                    .build(),
            )) else {
                continue;
            };
            // The configurations might not match if the target's configuration changed due to a
            // transition or trimming. Filter such targets, with one exception: if the target is
            // subject to a non-idempotent rule transition, we have to keep it once if the keys
            // requested above, which never have shouldApplyRuleTransition set to false, don't
            // cover it. This case is rare, so we optimize for it not being hit.
            if Some(&configuration_key)
                != target
                    .get_configured_target()
                    .get_configuration_key()
                    .as_ref()
            {
                let lookup_key = get_configured_target_key(&target);
                let target_key = as_configured_target_key(&lookup_key)
                    .expect("configured target key")
                    .clone();
                if target_key.should_apply_rule_transition()
                    || self
                        .get_value_from_key(&SkyKey::new(
                            ConfiguredTargetKey::builder()
                                .set_label(label.clone())
                                .set_configuration_key(target_key.get_configuration_key().cloned())
                                .build(),
                        ))
                        .is_some()
                {
                    continue;
                }
                let extra = extra_configured_target_keys.get_or_insert_with(HashSet::new);
                if !extra.insert(target_key) {
                    continue;
                }
            }
            ans.push(target);
        }
        if let Some(null_configured_target) = self.get_null_configured_target(label) {
            ans.push(null_configured_target);
        }
        ans
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:287 PostAnalysisQueryEnvironment.getFwdDeps
    pub fn get_fwd_deps_set(&self, targets: &[T]) -> Arc<dyn ThreadSafeMutableSet<T>> {
        // PORT: java-hash-order. Java's HashMap<SkyKey, T> iterates in hash order.
        let mut targets_by_key: IndexMap<SkyKey, T> = IndexMap::new();
        for target in targets {
            targets_by_key.insert(
                lookup_key_to_sky_key(&get_configured_target_key(target)),
                target.clone(),
            );
        }
        let mut direct_deps_input: Vec<(SkyKey, Vec<SkyKey>)> = Vec::new();
        for key in targets_by_key.keys() {
            // Java WalkableGraph.getDirectDeps(keys) only returns done nodes.
            if self.graph.get_value(key).is_none() && self.graph.get_exception(key).is_none() {
                continue;
            }
            direct_deps_input.push((key.clone(), self.graph.get_direct_deps(key)));
        }
        let direct_deps = self.targetify_values_map(&targets_by_key, &direct_deps_input);
        if targets_by_key.len() != direct_deps.len() {
            let missing_targets: Vec<String> = targets_by_key
                .keys()
                .filter(|k| !direct_deps.contains_key(*k))
                .map(|k| format!("{:?}", k))
                .collect();
            self.event_handler.handle(&Event::warn(format!(
                "Targets were missing from graph: [{}]",
                missing_targets.join(", ")
            )));
        }
        let result = self.create_set();
        for (key, deps) in &direct_deps {
            result.add_all(&self.filter_fwd_deps(&targets_by_key[key], deps));
        }
        result
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:314 PostAnalysisQueryEnvironment.filterFwdDeps
    fn filter_fwd_deps(
        &self,
        config_target: &T,
        raw_fwd_deps: &[ClassifiedDependency<T>],
    ) -> Vec<T> {
        if self.settings.is_empty() {
            return get_dependencies(raw_fwd_deps);
        }
        self.get_allowed_deps(config_target, raw_fwd_deps.to_vec())
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:323 PostAnalysisQueryEnvironment.getReverseDeps
    fn get_reverse_deps_impl(&self, targets: &[T]) -> Vec<T> {
        let mut targets_by_key: IndexMap<SkyKey, T> = IndexMap::new();
        for target in targets {
            targets_by_key.insert(
                lookup_key_to_sky_key(&get_configured_target_key(target)),
                target.clone(),
            );
        }
        let keys: Vec<SkyKey> = targets_by_key.keys().cloned().collect();
        let reverse_deps = self.skip_delegating_ancestors(self.graph.get_reverse_deps(&keys));
        let reverse_deps_by_key = self.targetify_values_map(&targets_by_key, &reverse_deps);
        if targets_by_key.len() != reverse_deps_by_key.len() {
            let missing_targets: Vec<String> = targets_by_key
                .keys()
                .filter(|k| !reverse_deps_by_key.contains_key(*k))
                .map(|k| format!("{:?}", k))
                .collect();
            self.event_handler.handle(&Event::warn(format!(
                "Targets were missing from graph: [{}]",
                missing_targets.join(", ")
            )));
        }
        let mut reverse_deps_by_ct: Vec<(T, Vec<ClassifiedDependency<T>>)> = Vec::new();
        for (key, deps) in reverse_deps_by_key {
            reverse_deps_by_ct.push((targets_by_key[&key].clone(), deps));
        }
        if reverse_deps_by_ct.is_empty() {
            Vec::new()
        } else {
            self.filter_reverse_deps(reverse_deps_by_ct)
        }
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:348 PostAnalysisQueryEnvironment.filterReverseDeps
    fn filter_reverse_deps(
        &self,
        raw_reverse_deps: Vec<(T, Vec<ClassifiedDependency<T>>)>,
    ) -> Vec<T> {
        let result = KeyedNodeSet::<T>::new();
        for (target, rdeps) in raw_reverse_deps {
            let mut rule_deps: Vec<ClassifiedDependency<T>> = Vec::new();
            for parent in rdeps {
                let dependency = parent.dependency.clone();
                if matches!(
                    parent.dependency.get_configured_target(),
                    ConfiguredTarget::Rule(_)
                ) && !self.dependency_filter_is_all_deps
                {
                    rule_deps.push(parent);
                } else {
                    result.add(dependency);
                }
            }
            for dep in self.get_allowed_deps(&target, rule_deps) {
                result.add(dep);
            }
        }
        result.to_vec()
    }

    /// Expands any delegating ancestors when computing reverse dependencies.
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:379 PostAnalysisQueryEnvironment.skipDelegatingAncestors
    fn skip_delegating_ancestors(
        &self,
        reverse_deps: Vec<(SkyKey, Vec<SkyKey>)>,
    ) -> Vec<(SkyKey, Vec<SkyKey>)> {
        let mut result: Vec<(SkyKey, Vec<SkyKey>)> = Vec::new();
        for (child, rdeps) in reverse_deps {
            let unwound_rdeps =
                self.unwind_reverse_dependency_delegation_layers_if_found(&child, &rdeps);
            result.push((child, unwound_rdeps.unwrap_or(rdeps)));
        }
        result
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:392 PostAnalysisQueryEnvironment.unwindReverseDependencyDelegationLayersIfFound
    fn unwind_reverse_dependency_delegation_layers_if_found(
        &self,
        child: &SkyKey,
        rdeps: &[SkyKey],
    ) -> Option<Vec<SkyKey>> {
        // Most rdeps will not be delegating. Performs an optimistic pass that avoids copying.
        let mut found_delegating_rdep = false;
        for rdep_key in rdeps {
            if rdep_key.function_name() != CONFIGURED_TARGET {
                continue;
            }
            let Some(rdep_value) = self.get_value_from_key(rdep_key) else {
                // Cannot find the actual value, possibly because it failed during analysis.
                continue;
            };
            let actual_parent_key = lookup_key_to_sky_key(&get_configured_target_key(&rdep_value));
            if actual_parent_key == *child {
                // The parent has the same value as the child because it is delegating.
                found_delegating_rdep = true;
                break;
            }
        }
        if !found_delegating_rdep {
            return None;
        }
        // PORT: java-hash-order (a HashSet of logical parents); insertion order.
        let mut logical_parents: IndexSet<SkyKey> = IndexSet::new();
        self.unwind_reverse_dependency_delegation_layers(child, rdeps, &mut logical_parents);
        Some(logical_parents.into_iter().collect())
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:421 PostAnalysisQueryEnvironment.unwindReverseDependencyDelegationLayers
    fn unwind_reverse_dependency_delegation_layers(
        &self,
        child: &SkyKey,
        rdeps: &[SkyKey],
        output: &mut IndexSet<SkyKey>,
    ) {
        // Checks the value of each rdep to see if it is delegating to `child`. If so, fetches
        // its rdeps and processes those, applying the same expansion as needed.
        for rdep_key in rdeps {
            if rdep_key.function_name() != CONFIGURED_TARGET {
                output.insert(rdep_key.clone());
                continue;
            }
            let Some(rdep_value) = self.get_value_from_key(rdep_key) else {
                // Cannot find the actual value, possibly because it failed during analysis.
                continue;
            };
            let actual_parent_key = lookup_key_to_sky_key(&get_configured_target_key(&rdep_value));
            if actual_parent_key != *child {
                output.insert(rdep_key.clone());
                continue;
            }
            // Otherwise `rdepKey` is delegating to child and needs to be unwound.
            let rdep_parents: Vec<SkyKey> = self
                .graph
                .get_reverse_deps(std::slice::from_ref(rdep_key))
                .into_iter()
                .next()
                .map(|(_, v)| v)
                .unwrap_or_default();
            // Applies this recursively in case there are multiple layers of delegation.
            self.unwind_reverse_dependency_delegation_layers(child, &rdep_parents, output);
        }
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:452 PostAnalysisQueryEnvironment.getAllowedDeps
    fn get_allowed_deps(&self, target: &T, mut deps: Vec<ClassifiedDependency<T>>) -> Vec<T> {
        // It's possible to query on a target that's configured in an exec configuration. In
        // those cases if --notool_deps is turned on, we only allow reachable targets that are
        // ALSO in an exec config. This is somewhat counterintuitive and subject to change in the
        // future but seems like the best option right now.
        if self.settings.contains(&Setting::OnlyTargetDeps) {
            let current_config = self.get_configuration(target);
            if current_config
                .as_ref()
                .is_some_and(|c| c.is_tool_configuration())
            {
                deps.retain(|dep| {
                    self.get_configuration(&dep.dependency)
                        .is_some_and(|c| c.is_tool_configuration())
                });
            } else {
                deps.retain(|dep| {
                    // We include source files, which have null configuration, even though they
                    // can also appear on exec-configured attributes like genrule#tools. While
                    // this may not be strictly correct, it's better to overapproximate than
                    // underapproximate the results.
                    match self.get_configuration(&dep.dependency) {
                        None => true,
                        Some(c) => !c.is_tool_configuration(),
                    }
                });
            }
        }
        if self.settings.contains(&Setting::NoImplicitDeps) {
            deps.retain(|dep| !dep.implicit);
        }
        get_dependencies(&deps)
    }

    /// Java `getRuleConfiguredTarget(T)`.
    fn get_rule_configured_target(target: &T) -> Option<&Arc<RuleConfiguredTarget>> {
        match target.get_configured_target() {
            ConfiguredTarget::Rule(r) => Some(r),
            _ => None,
        }
    }

    /// If `target` is an `OutputFileConfiguredTarget`, return the rule that produces it.
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:518 ConfiguredTargetQueryEnvironment.getOwningRuleforOutputConfiguredTarget
    fn get_owning_rule_for_output_configured_target(
        target: &T,
    ) -> Option<Arc<RuleConfiguredTarget>> {
        match target.get_configured_target() {
            ConfiguredTarget::OutputFile(o) => Some(o.get_generating_rule().clone()),
            _ => None,
        }
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:528 ConfiguredTargetQueryEnvironment.isAliasConfiguredTarget
    fn is_alias_configured_target(target: &T) -> bool {
        matches!(target.get_configured_target(), ConfiguredTarget::Alias(_))
    }

    /// Returns targetified dependencies wrapped as `ClassifiedDependency` objects which include
    /// information on if the target is an implicit or explicit dependency.
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:518 PostAnalysisQueryEnvironment.targetifyValues
    fn targetify_values(
        &self,
        parent: Option<&T>,
        dependencies: &[SkyKey],
        known_ct_deps: &mut HashSet<SkyKey>,
        resolved_aspect_classes: &mut HashSet<String>,
    ) -> Vec<ClassifiedDependency<T>> {
        let mut implicit_deps: Option<IndexSet<ConfiguredTargetKey>> = None;
        if let Some(parent) = parent {
            if let Some(rule_configured_target) = Self::get_rule_configured_target(parent) {
                implicit_deps = Some(rule_configured_target.get_implicit_deps().clone());
            }
        }

        let explicit_aspects = self.settings.contains(&Setting::IncludeAspects)
            && self.settings.contains(&Setting::ExplicitAspects);

        let mut values: Vec<ClassifiedDependency<T>> = Vec::new();
        // TODO(bazel-team): The end-goal approach is to treat aspects and toolchains as
        // first-class query nodes just like targets.
        for key in dependencies {
            if known_ct_deps.contains(key) {
                continue;
            }
            let function_name = key.function_name().get_name();
            if function_name == CONFIGURED_TARGET.get_name()
                || (explicit_aspects && function_name == "ASPECT")
            {
                let dependency = self.get_value_from_key(key);
                if dependency.is_none() && self.keep_going {
                    self.event_handler.handle(&Event::warn(format!(
                        "Dependency {:?} is unavaillable for querying. This may be because this \
                         query sets --keep_going, which continues analysis even some of the build \
                         graph  fails to analyze. For more accurate results ensure this request \
                         can build.",
                        key
                    )));
                    continue;
                }
                // If --keep_going wasn't applied and a build node is missing, something's wrong
                // with the build graph.
                let dependency = dependency.unwrap_or_else(|| {
                    panic!(
                        "IllegalStateException: query-requested node '{:?}' was unavailable in \
                         the query environment graph. If you come across this error, please file \
                         an issue at https://github.com/bazelbuild/bazel or contact the bazel \
                         configurability team.",
                        key
                    )
                });

                let implicit_dep = if key.downcast::<ConfiguredTargetKey>().is_none() {
                    // All non-configured target deps are implicit.
                    true
                } else if parent.is_some_and(Self::is_alias_configured_target) {
                    // Aliases have only one configured target dep: the ":actual" parameter.
                    false
                } else if let Some(owning_rule) =
                    parent.and_then(Self::get_owning_rule_for_output_configured_target)
                {
                    // An output file's generating target is an explicit dep.
                    match Self::get_rule_configured_target(&dependency) {
                        None => false,
                        Some(rule) => rule.base.lookup_key != owning_rule.base.lookup_key,
                    }
                } else if let Some(implicit_deps) = &implicit_deps {
                    // Check both the original guess key and the second correct key. In the case
                    // of the target platform, Util.findImplicitDeps also uses the original guess
                    // key.
                    let key_ctk = key.downcast::<ConfiguredTargetKey>().expect("checked");
                    implicit_deps.contains(key_ctk)
                        || as_configured_target_key(&get_configured_target_key(&dependency))
                            .is_some_and(|k| implicit_deps.contains(k))
                } else {
                    // No set of implicit deps available. Assume they're all implicit. This
                    // implies the parent isn't a rule configured target.
                    blaze_util::check_state!(
                        !parent.is_some_and(|p| Self::get_rule_configured_target(p).is_some())
                    );
                    true
                };

                values.push(ClassifiedDependency {
                    dependency,
                    implicit: implicit_dep,
                });
                known_ct_deps.insert(key.clone());
            } else if self.settings.contains(&Setting::IncludeAspects) && function_name == "ASPECT"
            {
                let _ = &resolved_aspect_classes;
                blaze_util::unported!(
                    "PostAnalysisQueryEnvironment.targetifyValues",
                    "aspect dependencies ({:?})",
                    key
                );
            } else if function_name == "TOOLCHAIN_RESOLUTION" {
                let deps = self.graph.get_direct_deps(key);
                values.extend(self.targetify_values(
                    None,
                    &deps,
                    known_ct_deps,
                    resolved_aspect_classes,
                ));
            }
        }
        values
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:632 PostAnalysisQueryEnvironment.targetifyValues
    fn targetify_values_map(
        &self,
        from_targets_by_key: &IndexMap<SkyKey, T>,
        input: &[(SkyKey, Vec<SkyKey>)],
    ) -> IndexMap<SkyKey, Vec<ClassifiedDependency<T>>> {
        let mut result: IndexMap<SkyKey, Vec<ClassifiedDependency<T>>> = IndexMap::new();
        for (from_key, deps) in input {
            result.insert(
                from_key.clone(),
                self.targetify_values(
                    from_targets_by_key.get(from_key),
                    deps,
                    &mut HashSet::new(),
                    &mut HashSet::new(),
                ),
            );
        }
        result
    }

    fn create_set(&self) -> Arc<dyn ThreadSafeMutableSet<T>> {
        Arc::new(KeyedNodeSet::<T>::new())
    }

    /// `getTargetsMatchingPattern` of both subclasses.
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:257 ConfiguredTargetQueryEnvironment.getTargetsMatchingPattern
    fn get_targets_matching_pattern_impl(
        &self,
        owner: &QueryExpression,
        pattern: &str,
        callback: &dyn Callback<T>,
    ) -> Result<(), QueryError> {
        let pattern_to_eval = match self.main_repo_target_parser.parse(pattern) {
            Ok(p) => p,
            Err(tpe) => {
                self.handle_error_impl(
                    owner,
                    tpe.get_message(),
                    Some(&target_parsing_detailed_exit_code(&tpe)),
                )?;
                return Ok(());
            }
        };
        let provider = GraphBackedRecursivePackageProvider {
            graph: &*self.graph,
            pkg_roots: self.pkg_path.get_path_entries().to_vec(),
        };
        let resolver = RecursivePackageProviderBackedTargetPatternResolver::new(
            &provider,
            self.event_handler.as_ref(),
            FilteringPolicy::NoFilter,
        );
        let repository = pattern_to_eval.get_repository();
        let ignored = self.get_ignored_subdirectories(&repository);
        let mut callback_error: Option<QueryError> = None;
        let eval_result = target_pattern_eval::eval(
            &pattern_to_eval,
            &resolver,
            &|| Ok(ignored.clone()),
            /* excluded_subdirectories= */ &[],
            &mut |partial_result: Vec<Target>| {
                if callback_error.is_some() {
                    return;
                }
                let mut transformed_result: Vec<T> = Vec::new();
                for target in &partial_result {
                    transformed_result
                        .extend(self.get_configured_targets_for_label(target.get_label()));
                }
                if let Err(e) = callback.process(&transformed_result) {
                    callback_error = Some(e);
                }
            },
        );
        if let Some(e) = callback_error {
            return Err(e);
        }
        match eval_result {
            Ok(()) => Ok(()),
            Err(TargetPatternEvalError::TargetParsing(exn)) => {
                self.handle_error_impl(
                    owner,
                    exn.get_message(),
                    Some(&target_parsing_detailed_exit_code(&exn)),
                )?;
                Ok(())
            }
            Err(TargetPatternEvalError::Interrupted) => {
                Err(QueryError::Interrupted(InterruptedException::new()))
            }
            Err(TargetPatternEvalError::ProcessPackageDirectory(message)) => Err(
                QueryException::new(Some(owner), message, query::Code::TargetNotInUniverseScope)
                    .into(),
            ),
            Err(other) => panic!(
                "IllegalStateException: unexpected target pattern evaluation failure: {:?}",
                other
            ),
        }
    }

    /// Handle an error during evaluation of `expression` by either throwing `QueryException` or
    /// emitting an event, depending on whether the evaluation is running in a "keep going" mode.
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:422 AbstractBlazeQueryEnvironment.handleError
    fn handle_error_impl(
        &self,
        expression: &QueryExpression,
        message: &str,
        detailed_exit_code: Option<&DetailedExitCode>,
    ) -> Result<(), QueryException> {
        if !self.keep_going {
            if let Some(detailed_exit_code) = detailed_exit_code {
                return Err(QueryException::with_failure_detail(
                    Some(expression),
                    message,
                    detailed_exit_code
                        .get_failure_detail()
                        .cloned()
                        .expect("DetailedExitCode without FailureDetail"),
                ));
            } else {
                // PORT: BugReport.sendBugReport("Undetailed failure: ...") skipped (bug reports).
                return Err(QueryException::new(
                    Some(expression),
                    message,
                    query::Code::NonDetailedError,
                ));
            }
        }
        self.event_handler
            .handle(&create_error_event(expression, message, detailed_exit_code));
        Ok(())
    }

    /// Evaluate the specified query expression in this environment, streaming results to the
    /// given `callback`. `callback.start()` will be called before query evaluation and
    /// `callback.close()` will be unconditionally called at the end of query evaluation.
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:160 AbstractBlazeQueryEnvironment.evaluateQueryInternal
    fn evaluate_query_internal(
        &self,
        expr: &QueryExpression,
        callback: &dyn ThreadSafeOutputFormatterCallback<T>,
    ) -> Result<QueryEvalResult, EvaluateQueryError>
    where
        Self: TargetAccessor<T>,
    {
        let empty_sensing_callback = EmptinessSensingCallback {
            callback,
            num_targets: AtomicI32::new(0),
            io_exception: IoExceptionSlot::new(),
        };
        // Target patterns are resolved on the fly so no pre-work to be done here
        // (PostAnalysisQueryEnvironment.preloadOrThrow).
        let mut io_exn: Option<IoException> = None;
        let mut fail_fast = true;
        let result = (|| -> Result<(), EvaluateQueryError> {
            empty_sensing_callback
                .start()
                .map_err(EvaluateQueryError::Io)?;
            // evalTopLevelInternal
            let env: &dyn QueryEnvironment<T> = self;
            match env
                .eval(
                    expr,
                    &QueryExpressionContext::empty(),
                    &empty_sensing_callback,
                )
                .get_checked()
            {
                Ok(()) => {}
                Err(QueryError::Query(e)) => {
                    return Err(QueryException::wrap_toplevel(e, expr).into());
                }
                Err(QueryError::Interrupted(e)) => return Err(e.into()),
                Err(QueryError::Cancellation) => {
                    panic!("java.util.concurrent.CancellationException")
                }
            }
            fail_fast = false;
            Ok(())
        })();
        // finally
        match empty_sensing_callback.close(fail_fast) {
            Ok(()) => {}
            // Only throw this IOException if we weren't about to throw a different exception.
            Err(OutputFormatterError::Io(e)) => io_exn = Some(e),
            Err(OutputFormatterError::Interrupted(e)) => return Err(e.into()),
        }
        result?;
        if let Some(io_exn) = io_exn {
            return Err(EvaluateQueryError::Io(io_exn));
        }

        let event_handler = &self.event_handler;
        if event_handler.has_errors() {
            let detailed_exit_code = event_handler.get_error_property();
            if !self.keep_going {
                if let Some(detailed_exit_code) = detailed_exit_code {
                    return Err(QueryException::with_failure_detail(
                        None,
                        format!(
                            "Evaluation of query \"{}\" failed",
                            expr.to_trunctated_string()
                        ),
                        detailed_exit_code
                            .get_failure_detail()
                            .cloned()
                            .expect("DetailedExitCode without FailureDetail"),
                    )
                    .into());
                }
                return Err(QueryException::new(
                    None,
                    format!(
                        "Evaluation of query \"{}\" failed due to BUILD file errors",
                        expr.to_trunctated_string()
                    ),
                    query::Code::BuildFileError,
                )
                .into());
            }
            event_handler.handle(&Event::warn(
                "--keep_going specified, ignoring errors. Results may be inaccurate",
            ));
            if let Some(detailed_exit_code) = detailed_exit_code {
                return Ok(QueryEvalResult::failure(
                    empty_sensing_callback.is_empty(),
                    detailed_exit_code,
                ));
            } else {
                return Ok(QueryEvalResult::failure(
                    empty_sensing_callback.is_empty(),
                    DetailedExitCode::of(FailureDetail {
                        message: format!(
                            "Evaluation of query \"{}\" failed due to BUILD file errors",
                            expr.to_trunctated_string()
                        ),
                        category: Some(Category::Query(Query {
                            code: query::Code::BuildFileError as i32,
                        })),
                    }),
                ));
            }
        }
        Ok(QueryEvalResult::success(empty_sensing_callback.is_empty()))
    }
}

/// Whether `constructDependencyFilter(settings)` returns `DependencyFilter.ALL_DEPS` itself.
// Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:117 AbstractBlazeQueryEnvironment.constructDependencyFilter
fn is_all_deps_filter(settings: &HashSet<Setting>) -> bool {
    // ONLY_TARGET_DEPS starts from ONLY_TARGET_DEPS; NO_IMPLICIT_DEPS and NO_NODEP_DEPS `and` a
    // new filter onto it; otherwise the ALL_DEPS constant is returned.
    !settings.contains(&Setting::OnlyTargetDeps)
        && !settings.contains(&Setting::NoImplicitDeps)
        && !settings.contains(&Setting::NoNodepDeps)
}

/// Java's `Setting` enum constant names.
fn setting_java_name(setting: &Setting) -> &'static str {
    match setting {
        Setting::TestsExpressionStrict => "TESTS_EXPRESSION_STRICT",
        Setting::NoImplicitDeps => "NO_IMPLICIT_DEPS",
        Setting::OnlyTargetDeps => "ONLY_TARGET_DEPS",
        Setting::NoNodepDeps => "NO_NODEP_DEPS",
        Setting::IncludeAspects => "INCLUDE_ASPECTS",
        Setting::ExplicitAspects => "EXPLICIT_ASPECTS",
    }
}

/// Check to make sure the settings requested are currently supported by this class.
// Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:203 PostAnalysisQueryEnvironment.checkSettings
fn check_settings(settings: &HashSet<Setting>) -> Result<(), QueryException> {
    if settings.contains(&Setting::NoNodepDeps)
        || settings.contains(&Setting::TestsExpressionStrict)
    {
        // Sets.difference(settings, {ONLY_TARGET_DEPS, NO_IMPLICIT_DEPS})
        // PORT: java-hash-order (a HashSet view); ordinal order here.
        let mut remaining: Vec<&Setting> = settings
            .iter()
            .filter(|s| **s != Setting::OnlyTargetDeps && **s != Setting::NoImplicitDeps)
            .collect();
        remaining.sort();
        return Err(QueryException::with_configurable_query_code(
            None,
            format!(
                "The following filter(s) are not currently supported by configured query: [{}]",
                remaining
                    .iter()
                    .map(|s| setting_java_name(s))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            configurable_query::Code::FiltersNotSupported,
        ));
    }
    Ok(())
}

/// `TargetParsingException.getDetailedExitCode()`.
fn target_parsing_detailed_exit_code(e: &TargetParsingException) -> DetailedExitCode {
    DetailedExitCode::of_exit_code(
        e.get_code().exit_code(),
        failure_details::target_parsing_failure_detail(e),
    )
}

// Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:584 AbstractBlazeQueryEnvironment.createErrorEvent
fn create_error_event(
    expr: &QueryExpression,
    message: &str,
    detailed_exit_code: Option<&DetailedExitCode>,
) -> Event {
    let event_message = format!(
        "Evaluation of query \"{}\" failed: {}",
        expr.to_trunctated_string(),
        message
    );
    let mut event = Event::error(event_message.clone());
    if let Some(detailed_exit_code) = detailed_exit_code {
        let mut failure_detail = detailed_exit_code
            .get_failure_detail()
            .cloned()
            .expect("DetailedExitCode without FailureDetail");
        failure_detail.message = event_message;
        event = event.with_property(Some(DetailedExitCode::of_exit_code(
            detailed_exit_code.get_exit_code(),
            failure_detail,
        )));
    }
    event
}

fn get_dependencies<T: Clone>(classified_dependencies: &[ClassifiedDependency<T>]) -> Vec<T> {
    classified_dependencies
        .iter()
        .map(|dep| dep.dependency.clone())
        .collect()
}

/// A callback counting the targets it forwards.
// Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:337 AbstractBlazeQueryEnvironment.EmptinessSensingCallback
struct EmptinessSensingCallback<'a, T> {
    callback: &'a dyn OutputFormatterCallback<T>,
    num_targets: AtomicI32,
    io_exception: IoExceptionSlot,
}

impl<T> EmptinessSensingCallback<'_, T> {
    fn is_empty(&self) -> bool {
        self.num_targets.load(Ordering::SeqCst) == 0
    }
}

impl<T> Callback<T> for EmptinessSensingCallback<'_, T> {
    fn process(&self, partial_result: &[T]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl<T> OutputFormatterCallback<T> for EmptinessSensingCallback<'_, T> {
    fn start(&self) -> Result<(), IoException> {
        self.callback.start()
    }

    fn process_output(&self, partial_result: &[T]) -> Result<(), OutputFormatterError> {
        self.num_targets
            .fetch_add(partial_result.len() as i32, Ordering::SeqCst);
        self.callback.process_output(partial_result)
    }

    fn close(&self, fail_fast: bool) -> Result<(), OutputFormatterError> {
        self.callback.close(fail_fast)
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }
}

/// Wrapper for evaluating query expression in a non-streaming blaze query environment.
// Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:224 AbstractBlazeQueryEnvironment.AbstractBlazeQueryEvaluateExpressionImpl
struct EvaluateExpressionImpl<'a, T> {
    env: &'a dyn QueryEnvironment<T>,
    expression: QueryExpression,
    context: QueryExpressionContext<T>,
    query_task_future_cancelled: Cell<Option<bool>>,
}

impl<T: Clone + 'static> EvaluateExpression<T> for EvaluateExpressionImpl<'_, T> {
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:236 AbstractBlazeQueryEvaluateExpressionImpl.eval
    fn eval(&self, callback: &dyn Callback<T>) -> QueryTaskFuture<()> {
        let query_task_future = self.env.eval(&self.expression, &self.context, callback);
        self.query_task_future_cancelled
            .set(Some(query_task_future.is_cancelled()));
        query_task_future
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:244 AbstractBlazeQueryEvaluateExpressionImpl.gracefullyCancel
    fn gracefully_cancel(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:251 AbstractBlazeQueryEvaluateExpressionImpl.isUngracefullyCancelled
    fn is_ungracefully_cancelled(&self) -> bool {
        self.query_task_future_cancelled.get().unwrap_or(false)
    }
}

impl<T: PostAnalysisNode> QueryEnvironment<T> for PostAnalysisQueryEnvironment<T>
where
    PostAnalysisQueryEnvironment<T>: TargetAccessor<T>,
{
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:728 PostAnalysisQueryEnvironment.getSiblingTargetsInPackage
    fn get_sibling_targets_in_package(&self, _target: &T) -> Result<Vec<T>, QueryError> {
        Err(QueryException::with_configurable_query_code(
            None,
            "siblings() not supported for post analysis queries",
            configurable_query::Code::SiblingsFunctionNotSupported,
        )
        .into())
    }

    fn get_targets_matching_pattern(
        &self,
        owner: &QueryExpression,
        pattern: &str,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        match self.get_targets_matching_pattern_impl(owner, pattern, callback) {
            Ok(()) => self.immediate_successful_future(()),
            Err(QueryError::Query(e)) => self.immediate_failed_future(e),
            Err(QueryError::Interrupted(_)) => self.immediate_cancelled_future(),
            Err(QueryError::Cancellation) => panic!("java.util.concurrent.CancellationException"),
        }
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:230 PostAnalysisQueryEnvironment.getOrCreate
    fn get_or_create(&self, target: &T) -> T {
        target.clone()
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:309 PostAnalysisQueryEnvironment.getFwdDeps
    fn get_fwd_deps(
        &self,
        targets: &[T],
        _context: &QueryExpressionContext<T>,
    ) -> Result<Vec<T>, InterruptedException> {
        Ok(self.get_fwd_deps_set(targets).to_vec())
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:323 PostAnalysisQueryEnvironment.getReverseDeps
    fn get_reverse_deps(
        &self,
        targets: &[T],
        _context: &QueryExpressionContext<T>,
    ) -> Result<Vec<T>, InterruptedException> {
        Ok(self.get_reverse_deps_impl(targets))
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:682 PostAnalysisQueryEnvironment.getTransitiveClosure
    fn get_transitive_closure(
        &self,
        targets: &dyn ThreadSafeMutableSet<T>,
        _context: &QueryExpressionContext<T>,
    ) -> Result<Arc<dyn ThreadSafeMutableSet<T>>, InterruptedException> {
        // Java: com/google/devtools/build/lib/query2/SkyQueryUtils.java:36 SkyQueryUtils.getTransitiveClosure
        let visited = self.create_set();
        let mut current: Vec<T> = targets.to_vec();
        while !current.is_empty() {
            let to_visit: Vec<T> = current
                .into_iter()
                .filter(|obj| !visited.contains(obj))
                .collect();
            current = self.get_fwd_deps_set(&to_visit).to_vec();
            for t in to_visit {
                visited.add(t);
            }
        }
        Ok(visited)
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:690 PostAnalysisQueryEnvironment.buildTransitiveClosure
    fn build_transitive_closure(
        &self,
        _caller: &QueryExpression,
        _target_nodes: &dyn ThreadSafeMutableSet<T>,
        _max_depth: Option<i32>,
    ) -> Result<(), QueryError> {
        // TODO(bazel-team): implement this. Just needed for error-checking.
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:696 PostAnalysisQueryEnvironment.getNodesOnPath
    fn get_nodes_on_path(
        &self,
        from: &T,
        to: &T,
        _context: &QueryExpressionContext<T>,
    ) -> Result<Vec<T>, InterruptedException> {
        // Java: com/google/devtools/build/lib/query2/SkyQueryUtils.java:63 SkyQueryUtils.getNodesOnPath
        let label = |t: &T| get_configured_target_key(t);
        // Tree of nodes visited so far.
        let mut node_to_parent: HashMap<ActionLookupKeyRef, Option<ActionLookupKeyRef>> =
            HashMap::new();
        let mut label_to_target: HashMap<ActionLookupKeyRef, T> = HashMap::new();
        // Contains all nodes left to visit in a (LIFO) stack.
        let mut to_visit: std::collections::VecDeque<T> = std::collections::VecDeque::new();
        to_visit.push_back(from.clone());
        node_to_parent.insert(label(from), None);
        label_to_target.insert(label(from), from.clone());
        while let Some(current) = to_visit.pop_front() {
            if label(to) == label(&current) {
                // Digraph.getPathToTreeNode(nodeToParent, label(to))
                let mut label_path: Vec<ActionLookupKeyRef> = Vec::new();
                let mut node = Some(label(to));
                while let Some(n) = node {
                    node = node_to_parent.get(&n).cloned().flatten();
                    label_path.push(n);
                }
                label_path.reverse();
                return Ok(label_path
                    .into_iter()
                    .map(|item| {
                        label_to_target
                            .get(&item)
                            .cloned()
                            .unwrap_or_else(|| panic!("NullPointerException: {}", item))
                    })
                    .collect());
            }
            for dep in self
                .get_fwd_deps_set(std::slice::from_ref(&current))
                .to_vec()
            {
                let dep_label = label(&dep);
                if !node_to_parent.contains_key(&dep_label) {
                    node_to_parent.insert(dep_label.clone(), Some(label(&current)));
                    label_to_target.insert(dep_label, dep.clone());
                    to_visit.push_front(dep);
                }
            }
        }
        // Note that the only current caller of this method checks first to see if there is a
        // path before calling this method. It is not clear what the return value should be here.
        // PORT: Java returns null; the Rust trait returns a list, so the empty list.
        Ok(Vec::new())
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:197 AbstractBlazeQueryEnvironment.eval
    fn eval(
        &self,
        expr: &QueryExpression,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        // Not all QueryEnvironment implementations embrace the async+streaming evaluation
        // framework. In particular, the streaming callbacks employed by functions like 'deps' use
        // buildTransitiveClosure. So if the implementation of that method does some heavyweight
        // blocking work, then it's best to do this blocking work in a single batch. Importantly,
        // the callback we pass in needs to maintain order.
        let aggregate_all_callback =
            query_util::new_ordered_aggregate_all_output_formatter_callback(self);
        let eval_all_future = expr.eval(
            self,
            context,
            &blaze_query_engine::callback_fn(|partial_result: &[T]| {
                aggregate_all_callback.process(partial_result)
            }),
        );
        self.when_succeeds_call(&eval_all_future, || {
            callback.process(&aggregate_all_callback.get_result())
        })
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:263 AbstractBlazeQueryEnvironment.createEvaluateExpression
    fn create_evaluate_expression<'a>(
        &'a self,
        expr: &QueryExpression,
        context: &QueryExpressionContext<T>,
    ) -> Box<dyn EvaluateExpression<T> + 'a> {
        Box::new(EvaluateExpressionImpl {
            env: self,
            expression: expr.clone(),
            context: context.clone(),
            query_task_future_cancelled: Cell::new(None),
        })
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:557 ConfiguredTargetQueryEnvironment.createThreadSafeMutableSet
    fn create_thread_safe_mutable_set(&self) -> Arc<dyn ThreadSafeMutableSet<T>> {
        self.create_set()
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:706 PostAnalysisQueryEnvironment.createUniquifier
    fn create_uniquifier(&self) -> Box<dyn Uniquifier<T>> {
        Box::new(UniquifierImpl::new(configured_target_key_extractor::<T>()))
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:711 PostAnalysisQueryEnvironment.createMinDepthUniquifier
    fn create_min_depth_uniquifier(&self) -> Box<dyn MinDepthUniquifier<T>> {
        Box::new(MinDepthUniquifierImpl::new(
            configured_target_key_extractor::<T>(),
            DEFAULT_THREAD_COUNT,
        ))
    }

    fn handle_error(
        &self,
        expression: &QueryExpression,
        message: &str,
        detailed_exit_code: Option<&DetailedExitCode>,
    ) -> Result<(), QueryException> {
        self.handle_error_impl(expression, message, detailed_exit_code)
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:721 PostAnalysisQueryEnvironment.getTransitiveLoadFilesHelper
    fn get_transitive_load_files_helper(
        &self,
    ) -> Result<Box<dyn TransitiveLoadFilesHelper<T> + '_>, QueryException> {
        Err(QueryException::with_configurable_query_code(
            None,
            "buildfiles() doesn't make sense for the configured target graph",
            configurable_query::Code::BuildfilesFunctionNotSupported,
        ))
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:502 AbstractBlazeQueryEnvironment.transitiveLoadFiles
    fn transitive_load_files(
        &self,
        _targets: &[T],
        _also_add_build_files: bool,
        _seen_packages: &Mutex<HashSet<PackageIdentifier>>,
        _seen_bzl_labels: &Mutex<HashSet<Label>>,
        _uniquifier: &dyn Uniquifier<T>,
        _helper: &dyn TransitiveLoadFilesHelper<T>,
        _callback: &dyn Callback<T>,
    ) -> Result<(), QueryError> {
        // PORT: unreachable: getTransitiveLoadFilesHelper always throws for post-analysis
        // queries, and the helper is required to call this.
        panic!("IllegalStateException: transitiveLoadFiles without a TransitiveLoadFilesHelper")
    }

    fn get_accessor(&self) -> &dyn TargetAccessor<T> {
        self
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:560 AbstractBlazeQueryEnvironment.isSettingEnabled
    fn is_setting_enabled(&self, setting: Setting) -> bool {
        self.settings.contains(&setting)
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:565 AbstractBlazeQueryEnvironment.getFunctions
    fn get_functions(&self) -> Vec<Arc<dyn QueryFunction<T>>> {
        let mut builder = default_query_functions::<T>();
        builder.extend(self.extra_functions.iter().cloned());
        builder
    }
}

// -------------------------------------------------------------------------------------------
// The graph lookups of the environment.

/// The graph and configurations a post-analysis environment looks configured targets up in
/// (shared with `ConfigFunction`, which Java gives the environment itself by a cast).
#[derive(Clone)]
pub struct PostAnalysisGraph {
    pub graph: Arc<dyn WalkableGraph>,
    pub top_level_configurations: Arc<TopLevelConfigurations>,
    pub transitive_configurations: Arc<IndexMap<String, Arc<BuildConfigurationValue>>>,
}

impl PostAnalysisGraph {
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:346 ConfiguredTargetQueryEnvironment.getValueFromKey
    pub fn get_value_from_key<T: PostAnalysisNode>(&self, key: &SkyKey) -> Option<T> {
        if key.function_name() != CONFIGURED_TARGET {
            blaze_util::unported!(
                "ConfiguredTargetQueryEnvironment.getValueFromKey",
                "aspect keys as query nodes ({:?})",
                key
            );
        }
        blaze_sky_top::skyframe_executor::get_configured_target_value(&*self.graph, key)
            .map(T::from_configured_target_value)
    }

    /// Returns the node for the given label and configuration if it exists, else `None`.
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:325 ConfiguredTargetQueryEnvironment.getConfiguredTarget
    pub fn get_configured_target<T: PostAnalysisNode>(
        &self,
        label: &Label,
        configuration: Option<&Arc<BuildConfigurationValue>>,
    ) -> Option<T> {
        let configuration_key: Option<BuildConfigurationKey> = configuration.map(|c| c.get_key());
        let target = self.get_value_from_key::<T>(&SkyKey::new(
            ConfiguredTargetKey::builder()
                .set_label(label.clone())
                .set_configuration_key(configuration_key.clone())
                .build(),
        ))?;
        // The configurations might not match if the target's configuration changed due to a
        // transition or trimming. Filters such targets.
        if configuration_key != target.get_configured_target().get_configuration_key() {
            return None;
        }
        Some(target)
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:483 ConfiguredTargetQueryEnvironment.getTargetConfiguredTarget
    pub fn get_target_configured_target<T: PostAnalysisNode>(&self, label: &Label) -> Option<T> {
        if self.top_level_configurations.is_top_level_target(label) {
            self.get_configured_target::<T>(
                label,
                self.top_level_configurations
                    .get_configuration_for_top_level_target(label),
            )
        } else {
            for configuration in self.top_level_configurations.get_configurations() {
                if let Some(to_return) =
                    self.get_configured_target::<T>(label, configuration.as_ref())
                {
                    return Some(to_return);
                }
            }
            None
        }
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:501 ConfiguredTargetQueryEnvironment.getNullConfiguredTarget
    pub fn get_null_configured_target<T: PostAnalysisNode>(&self, label: &Label) -> Option<T> {
        self.get_configured_target::<T>(label, None)
    }
}

// -------------------------------------------------------------------------------------------
// GraphBackedRecursivePackageProvider.

/// A `RecursivePackageProvider` backed by a `WalkableGraph`, used by post-analysis queries
/// (`UniverseTargetPattern.all()`, `RecursivePkgValueRootPackageExtractor`).
// Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:58 GraphBackedRecursivePackageProvider
pub struct GraphBackedRecursivePackageProvider<'g> {
    pub graph: &'g dyn WalkableGraph,
    pub pkg_roots: Vec<Root>,
}

impl GraphBackedRecursivePackageProvider<'_> {
    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:131 GraphBackedRecursivePackageProvider.getPackageInternal
    fn get_package_internal(
        &self,
        package_name: &PackageIdentifier,
    ) -> Result<Option<Arc<Package>>, NoSuchPackageException> {
        let key = SkyKey::new(PackageKey::key(package_name.clone()));
        if let Some(pkg_value) = self.graph.get_value(&key) {
            let pkg_value = pkg_value
                .downcast::<PackageValue>()
                .unwrap_or_else(|_| panic!("PACKAGE value"));
            return Ok(Some(pkg_value.get_package().clone()));
        }
        if let Some(e) = self.graph.get_exception(&key) {
            if let Some(nspe) = e.downcast_ref::<NoSuchPackageException>() {
                return Err(nspe.clone());
            }
            panic!("ClassCastException: {}", e);
        }
        if self.graph.is_cycle(&key) {
            return Err(NoSuchPackageException::new(
                package_name.clone(),
                "Package depends on a cycle",
            ));
        }
        Ok(None)
    }

    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:213 GraphBackedRecursivePackageProvider.bulkIsPackage
    fn bulk_is_package(
        &self,
        event_handler: &dyn ExtendedEventHandler,
        pkg_ids: &[PackageIdentifier],
    ) -> Vec<PackageIdentifier> {
        let mut result_builder: Vec<PackageIdentifier> = Vec::new();
        let mut non_successful_package_lookup_keys: Vec<SkyKey> = Vec::new();
        for pkg_id in pkg_ids {
            let key = SkyKey::new(PackageLookupKey::key(pkg_id.clone()));
            match self.graph.get_value(&key) {
                None => {
                    // Could be because the node legitimately isn't in the graph (which definitely
                    // implies the package doesn't exist) but could also be because the node
                    // exists but is in error. In the latter case, we want to print that error
                    // message out.
                    non_successful_package_lookup_keys.push(key);
                }
                Some(v) => {
                    let package_lookup_value = v
                        .downcast::<PackageLookupValue>()
                        .unwrap_or_else(|_| panic!("PACKAGE_LOOKUP value"));
                    if package_lookup_value.package_exists() {
                        result_builder.push(pkg_id.clone());
                    }
                }
            }
        }
        if !non_successful_package_lookup_keys.is_empty() {
            for (_, exception) in self
                .graph
                .get_missing_and_exceptions(&non_successful_package_lookup_keys)
            {
                if let Some(e) = exception
                    .as_ref()
                    .and_then(|e| e.downcast_ref::<NoSuchPackageException>())
                {
                    event_handler.handle(&Event::error(e.get_message()));
                }
            }
        }
        result_builder
    }

    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:266 GraphBackedRecursivePackageProvider.checkValidDirectoryAndGetRoots
    fn check_valid_directory_and_get_roots(&self, repository: &RepositoryName) -> Vec<Root> {
        // Check that this package is covered by at least one of our universe patterns.
        // UniverseTargetPattern.all(): every package is in the universe.
        if repository.is_main() {
            self.pkg_roots.clone()
        } else {
            match self
                .graph
                .get_value(&SkyKey::new(RepositoryDirectoryKey::key(
                    repository.clone(),
                )))
                .map(|v| {
                    v.downcast::<RepositoryDirectoryValue>()
                        .unwrap_or_else(|_| panic!("REPOSITORY_DIRECTORY value"))
                })
                .as_deref()
            {
                Some(RepositoryDirectoryValue::Success { root, .. }) => vec![root.clone()],
                // If this key doesn't exist, the repository is outside the universe, so we
                // return "nothing".
                _ => Vec::new(),
            }
        }
    }
}

impl RecursivePackageProvider for GraphBackedRecursivePackageProvider<'_> {
    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:119 GraphBackedRecursivePackageProvider.getPackage
    fn get_package(
        &self,
        _event_handler: &dyn ExtendedEventHandler,
        package_name: &PackageIdentifier,
    ) -> Result<Arc<Package>, ProviderError> {
        if let Some(pkg) = self.get_package_internal(package_name)? {
            return Ok(pkg);
        }
        // If the package key does not exist in the graph, then it must not correspond to any
        // package, because the SkyQuery environment has already loaded the universe.
        Err(NoSuchPackageException::build_file_not_found(
            package_name.clone(),
            "BUILD file not found on package path",
        )
        .into())
    }

    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:258 GraphBackedRecursivePackageProvider.isPackage
    fn is_package(
        &self,
        event_handler: &dyn ExtendedEventHandler,
        package_id: &PackageIdentifier,
    ) -> Result<bool, ProviderError> {
        Ok(!self
            .bulk_is_package(event_handler, std::slice::from_ref(package_id))
            .is_empty())
    }

    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:300 GraphBackedRecursivePackageProvider.streamPackagesUnderDirectory
    fn stream_packages_under_directory(
        &self,
        results: &mut dyn FnMut(Vec<PackageIdentifier>),
        _event_handler: &dyn ExtendedEventHandler,
        repository: &RepositoryName,
        directory: &PathFragment,
        ignored_subdirectories: &IgnoredSubdirectories,
        excluded_subdirectories: &[PathFragment],
    ) -> Result<(), ProviderError> {
        // Java: com/google/devtools/build/lib/skyframe/RecursivePkgValueRootPackageExtractor.java:36 RecursivePkgValueRootPackageExtractor.streamPackagesFromRoots
        let roots = self.check_valid_directory_and_get_roots(repository);
        let filtered_ignored_subdirectories =
            ignored_subdirectories.filter_for_directory(directory);

        for root in roots {
            let rooted_path = RootedPath::to_rooted_path(root, directory);
            let lookup = self.graph.get_value(&SkyKey::new(RecursivePkgKey::key(
                repository.clone(),
                rooted_path.clone(),
                filtered_ignored_subdirectories.clone(),
            )));
            let Some(lookup) = lookup else {
                // A null lookup should only happen during post-analysis queries which have
                // access to --universe_scope logic.
                // PORT: Java throws a QueryException (TARGET_NOT_IN_UNIVERSE_SCOPE); the provider
                // error carries its message (mapped back in get_targets_matching_pattern_impl).
                return Err(ProviderError::Eval(
                    TargetPatternEvalError::ProcessPackageDirectory(format!(
                        "Unable to load package '{}' because package is not in scope. Check that \
                         all target patterns in query expression are within the --universe_scope \
                         of this query.",
                        rooted_path
                    )),
                ));
            };
            let lookup = lookup
                .downcast::<RecursivePkgValue>()
                .unwrap_or_else(|_| panic!("RECURSIVE_PKG value"));
            let mut package_ids: Vec<PackageIdentifier> = Vec::new();
            for package_name in lookup.get_packages() {
                // TODO(bazel-team): Make RecursivePkgValue return NestedSet<PathFragment> so this
                // transform is unnecessary.
                let package_name_path_fragment = PathFragment::create(package_name);
                if !excluded_subdirectories
                    .iter()
                    .any(|e| package_name_path_fragment.starts_with(e))
                {
                    package_ids.push(PackageIdentifier::create(
                        repository.clone(),
                        package_name_path_fragment,
                    ));
                }
            }
            results(package_ids);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Java: src/test/java/com/google/devtools/build/lib/query2/PostAnalysisQueryTest.java
    // PORT: the PostAnalysisQueryTest suite drives analysis through BuildViewTestCase.
    #[test]
    #[ignore = "needs the BuildViewTestCase analysis harness"]
    fn test_target_literal_with_missing_targets() {}

    #[test]
    fn check_settings_rejects_nodep_filtering() {
        let settings: HashSet<Setting> = [Setting::NoNodepDeps, Setting::OnlyTargetDeps]
            .into_iter()
            .collect();
        let e = check_settings(&settings).unwrap_err();
        assert_eq!(
            e.get_message(),
            "The following filter(s) are not currently supported by configured query: [NO_NODEP_DEPS]"
        );
    }

    #[test]
    fn check_settings_accepts_supported_filters() {
        let settings: HashSet<Setting> = [Setting::OnlyTargetDeps, Setting::NoImplicitDeps]
            .into_iter()
            .collect();
        assert!(check_settings(&settings).is_ok());
    }
}
