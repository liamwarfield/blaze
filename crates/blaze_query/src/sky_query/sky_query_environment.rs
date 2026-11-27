//! Port of `query2/SkyQueryEnvironment.java` (with `query2/SkyQueryUtils.java`): a query
//! environment that evaluates over a Skyframe graph prepopulated with the universe
//! (`--universe_scope`, `--infer_universe_scope`) and supports `allrdeps` and `rbuildfiles`.
//!
//! PORT: Java evaluates the query on a thread pool (`queryEvaluationParallelismLevel` threads)
//! and streams results through a `BatchStreamedCallback`; Blaze evaluates on the calling thread
//! (the engine's futures are immediate). The results are the same; their order within the
//! streamed output is the order of the sequential visitation, where Java's depends on thread
//! scheduling (see `blaze_sky_loading::parallel_visitor`). The package `MultisetSemaphore` only
//! bounds memory and is not ported.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use blaze_cmdline::{
    Label, PackageIdentifier, SignedTargetPattern, TargetPattern, TargetPatternParser,
};
use blaze_events::{Event, ExtendedEventHandler};
use blaze_packages::exceptions::NoSuchPackageException;
use blaze_packages::package::Package;
use blaze_packages::target::{LabelPrinter, Target};
use blaze_protos::failure_details::{
    FailureDetail, PackageLoading, Query, failure_detail::Category, package_loading, query,
};
use blaze_query_engine::callback::{IoException, IoExceptionSlot, OutputFormatterError};
use blaze_query_engine::query_expression_mapper;
use blaze_query_engine::query_util::{
    MinDepthUniquifierImpl, ThreadSafeMutableKeyExtractorBackedSetImpl, UniquifierImpl,
};
use blaze_query_engine::streamable_query_environment::StreamableQueryEnvironment;
use blaze_query_engine::{
    AllRdepsFunction, Callback, DetailedExitCode, EvaluateExpression, InterruptedException,
    MinDepthUniquifier, OutputFormatterCallback, QueryEnvironment, QueryEnvironmentFutures,
    QueryError, QueryEvalResult, QueryException, QueryExpression, QueryExpressionContext,
    QueryFunction, QueryTaskFuture, QueryVisibility, Setting, TargetAccessor,
    TargetNotFoundException, ThreadSafeMutableSet, ThreadSafeOutputFormatterCallback,
    TransitiveLoadFilesHelper, Uniquifier,
};
use blaze_sky_loading::graph_backed_recursive_package_provider::{
    GraphBackedRecursivePackageProvider, UniverseTargetPattern,
};
use blaze_sky_loading::ignored_subdirectories::{
    IgnoredSubdirectoriesKey, IgnoredSubdirectoriesValue,
};
use blaze_sky_loading::package_function::{PackageKey, PackageValue};
use blaze_sky_loading::package_lookup::{PackageLookupKey, PackageLookupValue};
use blaze_sky_loading::prepare_deps::{self, TargetPatternSequence};
use blaze_sky_loading::target_pattern_eval::{self, TargetPatternEvalError};
use blaze_sky_loading::target_pattern_function::{
    FilteringPolicy, RecursivePackageProviderBackedTargetPatternResolver, TargetPatternKey,
};
use blaze_sky_loading::transitive_traversal::{
    TRANSITIVE_TRAVERSAL, TransitiveTraversalKey, TransitiveTraversalValue,
};
use blaze_skyframe::{EvaluationResult, Interrupted, SkyKey, WalkableGraph};
use blaze_vfs::{PathFragment, Root};
use indexmap::{IndexMap, IndexSet};

use crate::common::abstract_blaze_query_environment::{
    self as abstract_env, AbstractBlazeQueryEnvironment, AbstractBlazeQueryEnvironmentBase,
    AbstractBlazeQueryEvaluateExpressionImpl, EvaluateQueryError, GetTargetForQueryError,
    LabelFilter, PreloadError, target_key_extractor,
};
use crate::common::query_transitive_package_preloader;
use crate::common::universe_scope::UniverseScope;
use crate::compat::failure_details;
use crate::query::blaze_query_environment::new_fake_load_target;
use crate::query::blaze_target_accessor;
use crate::sky_query::expression_mappers::{
    FilteredDirectRdepsInUniverseExpressionMapper, RdepsToAllRdepsQueryExpressionMapper,
};
use crate::sky_query::parallel_sky_query_utils;
use crate::sky_query::rbuild_files::RBuildFilesFunction;

/// 10k is likely a good balance between using batch efficiently and not blowing up memory.
// Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:153 SkyQueryEnvironment.BATCH_CALLBACK_SIZE
pub const BATCH_CALLBACK_SIZE: usize = 10000;

/// Builds the graph of a universe and gives access to it (Java `WalkableGraphFactory`, the
/// `SkyframeExecutor`).
// Java: com/google/devtools/build/skyframe/WalkableGraph.java:110 WalkableGraph.WalkableGraphFactory
pub trait WalkableGraphFactory {
    /// Performs a graph update to make sure the transitive closure of `roots` is present in the
    /// graph, unconditionally in keep-going mode, and returns the result and the graph.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2689 SkyframeExecutor.prepareAndGet
    fn prepare_and_get(
        &self,
        roots: &[SkyKey],
        parallelism: i32,
        event_handler: Arc<dyn ExtendedEventHandler>,
    ) -> Result<(EvaluationResult, Arc<dyn WalkableGraph>), Interrupted>;
}

/// The `TRANSITIVE_TRAVERSAL` key of a label (Java: a `Label` is its own key).
// Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1410 SkyQueryEnvironment.TARGET_TO_SKY_KEY
pub fn label_key(label: &Label) -> SkyKey {
    SkyKey::new(TransitiveTraversalKey::key(label.clone()))
}

// Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1410 SkyQueryEnvironment.TARGET_TO_SKY_KEY
pub fn target_to_sky_key(target: &Target) -> SkyKey {
    label_key(target.get_label())
}

/// Java `IS_LABEL`.
// Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1344 SkyQueryEnvironment.IS_LABEL
pub fn is_label(key: &SkyKey) -> bool {
    key.function_name() == TRANSITIVE_TRAVERSAL
}

/// Java `SKYKEY_TO_LABEL`.
// Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1347 SkyQueryEnvironment.SKYKEY_TO_LABEL
pub fn sky_key_to_label(key: &SkyKey) -> Option<Label> {
    key.downcast::<TransitiveTraversalKey>()
        .map(|k| k.get_label().clone())
}

/// Java `Multimap<SkyKey, SkyKey> packageKeyToTargetKeyMap` (ArrayListMultimap; here keys in
/// first-seen order).
pub type PackageKeyToTargetKeyMap = IndexMap<PackageIdentifier, Vec<SkyKey>>;

// Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1353 SkyQueryEnvironment.makePackageKeyToTargetKeyMap
pub fn make_package_key_to_target_key_map<'a>(
    keys: impl IntoIterator<Item = &'a SkyKey>,
) -> PackageKeyToTargetKeyMap {
    let mut package_key_to_target_key_map: PackageKeyToTargetKeyMap = IndexMap::new();
    for key in keys {
        let Some(label) = sky_key_to_label(key) else {
            continue;
        };
        package_key_to_target_key_map
            .entry(label.get_package_identifier().clone())
            .or_default()
            .push(key.clone());
    }
    package_key_to_target_key_map
}

fn package_key(pkg_id: &PackageIdentifier) -> SkyKey {
    SkyKey::new(PackageKey::key(pkg_id.clone()))
}

/// An error message and a `FailureDetail` to include in either `handleError`'s thrown
/// `QueryException` or emitted `Event`.
// Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1278 SkyQueryEnvironment.ErrorsToHandle
struct ErrorsToHandle {
    message: String,
    failure_detail: FailureDetail,
}

/// A query environment over a Skyframe graph in which the universe was loaded.
// Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:149 SkyQueryEnvironment
pub struct SkyQueryEnvironment {
    base: AbstractBlazeQueryEnvironmentBase,
    loading_phase_threads: i32,
    graph_factory: Arc<dyn WalkableGraphFactory>,
    universe_scope: UniverseScope,
    main_repo_target_parser: TargetPatternParser,
    parser_prefix: PathFragment,
    pkg_path: Vec<Root>,
    query_evaluation_parallelism_level: i32,
    visibility_deps_are_allowed: bool,
    toolchain_type_deps_are_allowed: bool,
    // The following fields are set in the #beforeEvaluateQuery method.
    graph: RefCell<Option<Arc<dyn WalkableGraph>>>,
    graph_backed_recursive_package_provider:
        RefCell<Option<Arc<GraphBackedRecursivePackageProvider>>>,
}

impl SkyQueryEnvironment {
    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:204 SkyQueryEnvironment.<init>
    pub fn new(
        keep_going: bool,
        loading_phase_threads: i32,
        query_evaluation_parallelism_level: i32,
        event_handler: Arc<dyn ExtendedEventHandler>,
        settings: HashSet<Setting>,
        extra_functions: Vec<Arc<dyn QueryFunction<Target>>>,
        main_repo_target_parser: TargetPatternParser,
        parser_prefix: PathFragment,
        graph_factory: Arc<dyn WalkableGraphFactory>,
        universe_scope: UniverseScope,
        pkg_path: Vec<Root>,
        label_printer: LabelPrinter,
    ) -> Self {
        // In #getAllowedDeps we have special treatment of deps entailed by the `visibility`
        // attribute. Since this attribute is of the NODEP type, that means we need a special
        // implementation of NO_NODEP_DEPS.
        let visibility_deps_are_allowed = !settings.contains(&Setting::NoNodepDeps);
        // The "toolchains" parameter of rule definition should be treated as an implicit dep
        // despite not being represented by an attribute.
        let toolchain_type_deps_are_allowed = !settings.contains(&Setting::NoImplicitDeps);
        let base = AbstractBlazeQueryEnvironmentBase::new(
            keep_going,
            /* strict_scope= */ true,
            /* label_filter= */ LabelFilter::AllLabels,
            event_handler,
            settings,
            extra_functions,
            label_printer,
        );
        SkyQueryEnvironment {
            base,
            loading_phase_threads,
            graph_factory,
            universe_scope,
            main_repo_target_parser,
            parser_prefix,
            pkg_path,
            query_evaluation_parallelism_level,
            visibility_deps_are_allowed,
            toolchain_type_deps_are_allowed,
            graph: RefCell::new(None),
            graph_backed_recursive_package_provider: RefCell::new(None),
        }
    }

    pub(crate) fn graph(&self) -> Arc<dyn WalkableGraph> {
        self.graph
            .borrow()
            .clone()
            .expect("the graph is set by beforeEvaluateQuery")
    }

    fn provider(&self) -> Arc<GraphBackedRecursivePackageProvider> {
        self.graph_backed_recursive_package_provider
            .borrow()
            .clone()
            .expect("the provider is set by beforeEvaluateQuery")
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:262 SkyQueryEnvironment.beforeEvaluateQuery
    fn before_evaluate_query(&self, expr: &QueryExpression) -> Result<(), QueryError> {
        let universe_key = self
            .universe_scope
            .get_universe_key(Some(expr), &self.parser_prefix);
        let universe_scope_list_to_use = universe_key.get_patterns().to_vec();
        // PORT: skipped, logging ("Using a --universe_scope value of %s").
        // Java getGraphRootsFromUniverseKeyAndExpression: the universe key.
        let roots = vec![SkyKey::new(universe_key.clone())];

        // PORT: skipped, profiling ("evaluation and walkable graph").
        let (result, graph) = self
            .graph_factory
            .prepare_and_get(
                &roots,
                self.loading_phase_threads,
                self.base.event_handler().clone(),
            )
            .map_err(|_| QueryError::Interrupted(InterruptedException::new()))?;

        // Java: `graph == null || graph != result.getWalkableGraph()`; every evaluation makes a
        // new walkable graph.
        check_evaluation_result(
            &universe_scope_list_to_use,
            &roots,
            &universe_key,
            &result,
            expr,
        )?;
        let provider = GraphBackedRecursivePackageProvider::new(
            graph.clone(),
            UniverseTargetPattern::Of(self.get_target_patterns_for_universe_key(&universe_key)),
            self.pkg_path.clone(),
        );
        *self.graph.borrow_mut() = Some(graph);
        *self.graph_backed_recursive_package_provider.borrow_mut() = Some(Arc::new(provider));
        Ok(())
    }

    /// Returns the TargetPatterns corresponding to `universe_key`.
    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:313 SkyQueryEnvironment.getTargetPatternsForUniverseKey
    fn get_target_patterns_for_universe_key(
        &self,
        universe_key: &TargetPatternSequence,
    ) -> Vec<TargetPattern> {
        prepare_deps::get_target_pattern_keys(&prepare_deps::get_sky_keys(
            universe_key,
            &**self.base.event_handler(),
            self.main_repo_target_parser.get_repo_mapping(),
        ))
        .iter()
        .map(|k| k.get_parsed_pattern().clone())
        .collect()
    }

    /// Returns true if this environment has a dependency filter on any edges.
    ///
    /// PORT: Java compares `dependencyFilter` with `DependencyFilter.ALL_DEPS` by identity; it is
    /// that constant exactly when no setting adds a filter (`constructDependencyFilter`).
    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:332 SkyQueryEnvironment.hasDependencyFilter
    pub fn has_dependency_filter(&self) -> bool {
        self.base.settings.contains(&Setting::OnlyTargetDeps)
            || self.base.settings.contains(&Setting::NoImplicitDeps)
            || self.base.settings.contains(&Setting::NoNodepDeps)
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:389 SkyQueryEnvironment.getQueryExpressionMapper
    fn get_query_expression_mapper(
        &self,
    ) -> Option<query_expression_mapper::ComposedQueryExpressionMapper<()>> {
        let constant_universe_scope_list = self.universe_scope.get_constant_value_maybe()?;
        if constant_universe_scope_list.len() != 1 {
            return None;
        }
        let universe_scope_pattern_string = &constant_universe_scope_list[0];
        let absolute_universe_scope_pattern = self
            .main_repo_target_parser
            .parse(
                &self
                    .main_repo_target_parser
                    .absolutize(universe_scope_pattern_string),
            )
            .ok()?;
        Some(query_expression_mapper::compose(vec![
            Box::new(RdepsToAllRdepsQueryExpressionMapper::new(
                self.main_repo_target_parser.clone(),
                absolute_universe_scope_pattern.clone(),
            )),
            Box::new(FilteredDirectRdepsInUniverseExpressionMapper::new(
                self.main_repo_target_parser.clone(),
                absolute_universe_scope_pattern,
            )),
        ]))
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:480 SkyQueryEnvironment.targetifyValues
    fn targetify_values(
        &self,
        input: &[(SkyKey, Vec<SkyKey>)],
        package_key_to_target_key_map: &PackageKeyToTargetKeyMap,
        missing_target_collector: &mut Vec<SkyKey>,
    ) -> Vec<(SkyKey, Vec<Target>)> {
        let all_targets = self.get_target_key_to_target_map_for_package_key_to_target_key_map(
            package_key_to_target_key_map,
        );
        let mut result = Vec::with_capacity(input.len());
        for (key, sky_keys) in input {
            // CompactHashSet<Target>: insertion order, one per target.
            let mut targets: Vec<Target> = Vec::with_capacity(sky_keys.len());
            let mut seen: HashSet<Label> = HashSet::new();
            for sky_key in sky_keys {
                match all_targets.get(sky_key) {
                    Some(target) => {
                        if seen.insert(target.get_label().clone()) {
                            targets.push(target.clone());
                        }
                    }
                    None => {
                        if !missing_target_collector.contains(sky_key) {
                            missing_target_collector.push(sky_key.clone());
                        }
                    }
                }
            }
            result.push((key.clone(), targets));
        }
        result
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:480 SkyQueryEnvironment.targetifyValues
    fn targetify_values_simple(
        &self,
        input: &[(SkyKey, Vec<SkyKey>)],
        missing_target_collector: &mut Vec<SkyKey>,
    ) -> Vec<(SkyKey, Vec<Target>)> {
        let mut all_keys: IndexSet<SkyKey> = IndexSet::new();
        for (_, v) in input {
            all_keys.extend(v.iter().cloned());
        }
        let map = make_package_key_to_target_key_map(&all_keys);
        self.targetify_values(input, &map, missing_target_collector)
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:516 SkyQueryEnvironment.getRawReverseDeps
    fn get_raw_reverse_deps(
        &self,
        transitive_traversal_keys: &[SkyKey],
    ) -> Vec<(SkyKey, Vec<Target>)> {
        self.targetify_values_simple(
            &self.get_reverse_dep_labels_of_labels(transitive_traversal_keys),
            &mut Vec::new(),
        )
    }

    /// The reverse deps of the given label keys in the graph.
    ///
    /// PORT: Java adds `extraGlobalDeps` (always empty in Bazel's `QueryExpressionContext`).
    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:525 SkyQueryEnvironment.getReverseDepLabelsOfLabels
    pub fn get_reverse_dep_labels_of_labels(
        &self,
        labels: &[SkyKey],
    ) -> Vec<(SkyKey, Vec<SkyKey>)> {
        self.graph().get_reverse_deps(labels)
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:539 SkyQueryEnvironment.getAllowedDeps
    fn get_allowed_deps(&self, rule: &blaze_packages::target::Rule) -> HashSet<Label> {
        let mut allowed_labels: HashSet<Label> = rule
            .get_transitions(&self.base.dependency_filter)
            .into_iter()
            .map(|(_, l)| l)
            .collect();
        if self.visibility_deps_are_allowed {
            // Rule#getTransitions only visits the labels of attribute values, so that means it
            // doesn't know about deps from the labels of the rule's package's
            // default_visibility. Therefore, we need to explicitly handle that here.
            allowed_labels.extend(rule.get_visibility_dependency_labels());
        }
        if self.toolchain_type_deps_are_allowed {
            for toolchain_type_requirement in rule.get_rule_class_object().get_toolchain_types() {
                allowed_labels.insert(toolchain_type_requirement.toolchain_type.clone());
            }
        }
        // We should add deps from aspects, otherwise they are going to be filtered out.
        allowed_labels.extend(rule.get_aspect_labels_superset(&self.base.dependency_filter));
        allowed_labels
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:558 SkyQueryEnvironment.filterFwdDeps
    fn filter_fwd_deps(&self, target: &Target, raw_fwd_deps: Vec<Target>) -> Vec<Target> {
        let Some(rule) = target.as_rule() else {
            return raw_fwd_deps;
        };
        let allowed_labels = self.get_allowed_deps(rule);
        raw_fwd_deps
            .into_iter()
            .filter(|t| allowed_labels.contains(t.get_label()))
            .collect()
    }

    /// The forward deps of `targets`, as a set; keys without a target go to
    /// `missing_target_collector`.
    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:567 SkyQueryEnvironment.getFwdDeps
    pub(crate) fn get_fwd_deps_collecting(
        &self,
        targets: &[Target],
        missing_target_collector: &mut Vec<SkyKey>,
    ) -> Arc<dyn ThreadSafeMutableSet<Target>> {
        // Maps.newHashMapWithExpectedSize: one target per key.
        let mut targets_by_key: IndexMap<SkyKey, Target> = IndexMap::new();
        for target in targets {
            targets_by_key.insert(target_to_sky_key(target), target.clone());
        }
        let keys: Vec<SkyKey> = targets_by_key.keys().cloned().collect();
        let fwd_dep_labels = self.get_fwd_dep_labels(&keys);
        let direct_deps = self.targetify_values_simple(&fwd_dep_labels, missing_target_collector);
        let result = self.create_thread_safe_mutable_set();
        for (key, deps) in direct_deps {
            let target = &targets_by_key[&key];
            for t in self.filter_fwd_deps(target, deps) {
                result.add(t);
            }
        }
        result
    }

    /// The label keys among the graph deps of `target_labels` (Java `getFwdDepLabels`).
    ///
    /// PORT: Java adds `extraGlobalDeps` (always empty in Bazel).
    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:600 SkyQueryEnvironment.getFwdDepLabels
    pub fn get_fwd_dep_labels(&self, target_labels: &[SkyKey]) -> Vec<(SkyKey, Vec<SkyKey>)> {
        blaze_util::check_state!(
            target_labels.iter().all(is_label),
            "Expected all labels: {:?}",
            target_labels
        );
        let graph = self.graph();
        // Java graph.getDirectDeps(keys): a map of the done keys.
        let mut results = Vec::with_capacity(target_labels.len());
        for key in target_labels {
            if graph.get_value(key).is_none() && graph.get_exception(key).is_none() {
                // PORT: DelegatingWalkableGraph.getDirectDeps skips keys not in the graph.
                continue;
            }
            let deps_labels: Vec<SkyKey> = graph
                .get_direct_deps(key)
                .into_iter()
                .filter(is_label)
                .collect();
            results.push((key.clone(), deps_labels));
        }
        results
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:786 SkyQueryEnvironment.filterRawReverseDepsOfTransitiveTraversalKeys
    pub(crate) fn filter_raw_reverse_deps_of_transitive_traversal_keys(
        &self,
        raw_reverse_deps: &[(SkyKey, Vec<SkyKey>)],
        package_key_to_target_key_map: &PackageKeyToTargetKeyMap,
    ) -> Vec<Target> {
        self.process_raw_reverse_deps(&self.targetify_values(
            raw_reverse_deps,
            package_key_to_target_key_map,
            &mut Vec::new(),
        ))
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:794 SkyQueryEnvironment.processRawReverseDeps
    fn process_raw_reverse_deps(&self, raw_reverse_deps: &[(SkyKey, Vec<Target>)]) -> Vec<Target> {
        // CompactHashSet: insertion order.
        let mut result: IndexMap<Label, Target> = IndexMap::new();
        let mut visited: HashSet<Label> = HashSet::new();
        let keys: HashSet<Label> = raw_reverse_deps
            .iter()
            .filter_map(|(k, _)| sky_key_to_label(k))
            .collect();
        for (_, parent_collection) in raw_reverse_deps {
            for parent in parent_collection {
                if visited.insert(parent.get_label().clone()) {
                    match parent.as_rule() {
                        Some(rule) if self.has_dependency_filter() => {
                            for label in self.get_allowed_deps(rule) {
                                if keys.contains(&label) {
                                    result
                                        .entry(parent.get_label().clone())
                                        .or_insert_with(|| parent.clone());
                                }
                            }
                        }
                        _ => {
                            result
                                .entry(parent.get_label().clone())
                                .or_insert_with(|| parent.clone());
                        }
                    }
                }
            }
        }
        result.into_values().collect()
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1120 SkyQueryEnvironment.getPackage
    fn get_package(
        &self,
        package_identifier: &PackageIdentifier,
    ) -> Result<Arc<Package>, GetPackageError> {
        let graph = self.graph();
        let key = package_key(package_identifier);
        if let Some(package_value) = graph
            .get_value(&key)
            .and_then(|v| v.downcast::<PackageValue>().ok())
        {
            let pkg = package_value.get_package();
            if pkg.contains_errors() {
                return Err(GetPackageError::NoSuchPackage(
                    NoSuchPackageException::build_file_contains_errors(package_identifier.clone()),
                ));
            }
            return Ok(pkg.clone());
        }
        if let Some(exception) = graph.get_exception(&key)
            && let Some(e) = exception.downcast_ref::<NoSuchPackageException>()
        {
            return Err(GetPackageError::NoSuchPackage(e.clone()));
        }
        if graph.is_cycle(&key) {
            Err(GetPackageError::NoSuchPackage(NoSuchPackageException::new(
                package_identifier.clone(),
                "Package depends on a cycle",
            )))
        } else {
            Err(GetPackageError::Query(QueryException::new(
                None,
                format!("{} does not exist in graph", package_identifier),
                query::Code::Cycle,
            )))
        }
    }

    /// Batch version of `getTarget`: missing targets are absent (Java `getTargets`).
    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1156 SkyQueryEnvironment.getTargets
    pub(crate) fn get_targets_by_label(&self, labels: &[Label]) -> Vec<(Label, Target)> {
        if labels.is_empty() {
            return Vec::new();
        }
        let mut package_id_to_label_map: IndexMap<PackageIdentifier, Vec<Label>> = IndexMap::new();
        for label in labels {
            package_id_to_label_map
                .entry(label.get_package_identifier().clone())
                .or_default()
                .push(label.clone());
        }
        let package_id_to_package_map =
            self.bulk_get_packages(&package_id_to_label_map.keys().cloned().collect::<Vec<_>>());
        let mut result_builder: Vec<(Label, Target)> = Vec::new();
        for (pkg_id, pkg_labels) in &package_id_to_label_map {
            let Some(pkg) = package_id_to_package_map.get(pkg_id) else {
                continue;
            };
            for label in pkg_labels {
                let Ok(target) = pkg.get_target(label.get_name()) else {
                    continue;
                };
                if result_builder.iter().any(|(l, _)| l == label) {
                    panic!("IllegalArgumentException: Multiple entries with same key: {label}");
                }
                result_builder.push((label.clone(), target.clone()));
            }
        }
        result_builder
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1190 SkyQueryEnvironment.bulkGetPackages
    pub(crate) fn bulk_get_packages(
        &self,
        pkg_ids: &[PackageIdentifier],
    ) -> HashMap<PackageIdentifier, Arc<Package>> {
        let keys: Vec<SkyKey> = pkg_ids.iter().map(package_key).collect();
        let mut pkg_results = HashMap::new();
        for (key, value) in self.graph().get_successful_values(&keys) {
            let pkg_id = key
                .downcast::<PackageKey>()
                .expect("a PACKAGE key")
                .0
                .clone();
            let pkg_value = value
                .downcast::<PackageValue>()
                .unwrap_or_else(|_| panic!("a PackageValue for {}", pkg_id));
            pkg_results.insert(pkg_id, pkg_value.get_package().clone());
        }
        pkg_results
    }

    /// Reports targets whose TRANSITIVE_TRAVERSAL failed or is missing.
    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1223 SkyQueryEnvironment.reportUnsuccessfulOrMissingTargets
    pub(crate) fn report_unsuccessful_or_missing_targets(
        &self,
        keys_with_targets: &[(Label, Target)],
        all_target_keys: &[SkyKey],
        caller: &QueryExpression,
    ) -> Result<(), QueryError> {
        let keys_found: HashSet<SkyKey> = keys_with_targets
            .iter()
            .map(|(l, _)| label_key(l))
            .collect();
        let mut missing_targets: Vec<SkyKey> = Vec::new();
        for key in all_target_keys {
            if !keys_found.contains(key) && !missing_targets.contains(key) {
                missing_targets.push(key.clone());
            }
        }
        let targets: Vec<Target> = keys_with_targets.iter().map(|(_, t)| t.clone()).collect();
        self.report_unsuccessful_or_missing_targets_internal(&targets, &missing_targets, caller)
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1238 SkyQueryEnvironment.reportUnsuccessfulOrMissingTargetsInternal
    pub(crate) fn report_unsuccessful_or_missing_targets_internal(
        &self,
        targets: &[Target],
        missing_target_keys: &[SkyKey],
        caller: &QueryExpression,
    ) -> Result<(), QueryError> {
        // Targets can be in four states:
        //  (1) Existent TransitiveTraversalValue with no error
        //  (2) Existent TransitiveTraversalValue with an error (eg. transitive dependency error)
        //  (3) Non-existent because it threw a SkyFunctionException
        //  (4) Non-existent because it was never evaluated
        //
        // We first find the errors in the existent TransitiveTraversalValues that have an error.
        // We then find which keys correspond to SkyFunctionExceptions and extract the errors from
        // those. Lastly, any leftover keys are marked as missing from the graph and an error is
        // produced.
        let mut errors_builder: Vec<ErrorsToHandle> = Vec::new();
        let successful_keys = self.filter_successfully_loaded_targets(targets, &mut errors_builder);

        let keys_with_target: Vec<SkyKey> = targets.iter().map(target_to_sky_key).collect();
        // Next, look for errors from the unsuccessfully evaluated TransitiveTraversal
        // skyfunctions.
        let mut unsuccessful_or_missing_keys: Vec<SkyKey> = keys_with_target
            .into_iter()
            .filter(|k| !successful_keys.contains(k))
            .collect();
        unsuccessful_or_missing_keys.extend(missing_target_keys.iter().cloned());
        self.process_unsuccessful_and_missing_keys(
            &unsuccessful_or_missing_keys,
            &mut errors_builder,
        );

        // Lastly, report all found errors.
        if !unsuccessful_or_missing_keys.is_empty() {
            self.base.event_handler().handle(&Event::warn(format!(
                "Targets were missing from graph: {}",
                iterables_to_string(&unsuccessful_or_missing_keys)
            )));
        }
        for error in errors_builder {
            self.base.handle_error(
                caller,
                &error.message,
                Some(&DetailedExitCode::of(error.failure_detail)),
            )?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1290 SkyQueryEnvironment.processUnsuccessfulAndMissingKeys
    fn process_unsuccessful_and_missing_keys(
        &self,
        unsuccessful_keys: &[SkyKey],
        errors_builder: &mut Vec<ErrorsToHandle>,
    ) {
        for (_, exception) in self.graph().get_missing_and_exceptions(unsuccessful_keys) {
            if let Some(exception) = exception {
                errors_builder.push(ErrorsToHandle {
                    message: exception.to_string(),
                    failure_detail: create_unsuccessful_key_failure(&exception),
                });
            }
        }
    }

    /// Filters for successful targets while storing error messages of unsuccessful targets.
    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1314 SkyQueryEnvironment.filterSuccessfullyLoadedTargets
    fn filter_successfully_loaded_targets(
        &self,
        targets: &[Target],
        errors_builder: &mut Vec<ErrorsToHandle>,
    ) -> HashSet<SkyKey> {
        let transitive_traversal_keys: Vec<SkyKey> =
            targets.iter().map(target_to_sky_key).collect();
        // First, look for errors in the successfully evaluated TransitiveTraversalValues. They may
        // have encountered errors that they were able to recover from.
        let mut successful_keys = HashSet::new();
        for (key, value) in self
            .graph()
            .get_successful_values(&transitive_traversal_keys)
        {
            let value = value
                .downcast::<TransitiveTraversalValue>()
                .expect("a TransitiveTraversalValue");
            if let Some(error_message) = value.get_error_message() {
                errors_builder.push(ErrorsToHandle {
                    message: error_message.to_owned(),
                    failure_detail: FailureDetail {
                        message: error_message.to_owned(),
                        category: Some(Category::Query(Query {
                            code: query::Code::SkyqueryTransitiveTargetError as i32,
                        })),
                    },
                });
            }
            successful_keys.insert(key);
        }
        successful_keys
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1373 SkyQueryEnvironment.getTargetKeyToTargetMapForPackageKeyToTargetKeyMap
    pub(crate) fn get_target_key_to_target_map_for_package_key_to_target_key_map(
        &self,
        package_key_to_target_key_map: &PackageKeyToTargetKeyMap,
    ) -> IndexMap<SkyKey, Target> {
        let mut result_builder = IndexMap::new();
        let mut processed_targets: HashSet<SkyKey> = HashSet::new();
        let pkg_ids: Vec<PackageIdentifier> =
            package_key_to_target_key_map.keys().cloned().collect();
        let package_keys: Vec<SkyKey> = pkg_ids.iter().map(package_key).collect();
        for (key, value) in self.graph().get_successful_values(&package_keys) {
            let pkg_id = &key.downcast::<PackageKey>().expect("a PACKAGE key").0;
            let pkg = value
                .downcast::<PackageValue>()
                .expect("a PackageValue")
                .get_package()
                .clone();
            for target_key in &package_key_to_target_key_map[pkg_id] {
                if processed_targets.insert(target_key.clone()) {
                    let label = sky_key_to_label(target_key).expect("a label key");
                    // Skip missing target.
                    if let Ok(target) = pkg.get_target(label.get_name()) {
                        result_builder.insert(target_key.clone(), target.clone());
                    }
                }
            }
        }
        result_builder
    }

    /// The BUILD file targets of the packages in `pkg_ids` that are in the graph.
    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1427 SkyQueryEnvironment.getBuildFileTargetsForPackageKeys
    pub(crate) fn get_build_file_targets_for_package_keys(
        &self,
        pkg_ids: &[PackageIdentifier],
    ) -> Vec<Target> {
        let keys: Vec<SkyKey> = pkg_ids.iter().map(package_key).collect();
        self.graph()
            .get_successful_values(&keys)
            .into_iter()
            .map(|(_, v)| {
                Target::InputFile(
                    v.downcast::<PackageValue>()
                        .expect("a PackageValue")
                        .get_package()
                        .get_build_file()
                        .clone(),
                )
            })
            .collect()
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1022 SkyQueryEnvironment.evalTargetPatternKey
    fn eval_target_pattern_key(
        &self,
        owner: &QueryExpression,
        target_pattern_key: &TargetPatternKey,
        callback: &dyn Callback<Target>,
    ) -> Result<(), QueryError> {
        let pattern_to_eval = target_pattern_key.get_parsed_pattern();
        let provider = self.provider();
        let event_handler = self.base.event_handler().clone();
        // Java makeNewTargetPatternResolver: NO_FILTER (the key's policy is applied below).
        let resolver = RecursivePackageProviderBackedTargetPatternResolver::new(
            &*provider,
            &*event_handler,
            FilteringPolicy::NoFilter,
        );
        let graph = self.graph();
        let repository = pattern_to_eval.get_repository();
        let mut callback_error: Option<QueryError> = None;
        let policy = target_pattern_key.get_policy().clone();
        let result = target_pattern_eval::eval(
            pattern_to_eval,
            &resolver,
            &|| {
                Ok(graph
                    .get_value(&SkyKey::new(IgnoredSubdirectoriesKey::key(
                        repository.clone(),
                    )))
                    .and_then(|v| v.downcast::<IgnoredSubdirectoriesValue>().ok())
                    .expect("NullPointerException: IgnoredSubdirectoriesValue not in graph")
                    .as_ignored_subdirectories()
                    .clone())
            },
            target_pattern_key.get_excluded_subdirectories(),
            &mut |targets: Vec<Target>| {
                if callback_error.is_some() {
                    return;
                }
                let targets: Vec<Target> = if policy == FilteringPolicy::NoFilter {
                    targets
                } else {
                    targets
                        .into_iter()
                        .filter(|t| policy.should_retain(t, /* explicit= */ false))
                        .collect()
                };
                if let Err(e) = callback.process(&targets) {
                    callback_error = Some(e);
                }
            },
        );
        if let Some(e) = callback_error {
            return Err(e);
        }
        match result {
            Ok(()) => Ok(()),
            Err(TargetPatternEvalError::TargetParsing(exn)) => {
                self.base.handle_error(
                    owner,
                    exn.get_message(),
                    Some(&DetailedExitCode::of(
                        failure_details::target_parsing_failure_detail(&exn),
                    )),
                )?;
                Ok(())
            }
            Err(TargetPatternEvalError::Interrupted) => {
                Err(QueryError::Interrupted(InterruptedException::new()))
            }
            Err(e) => panic!("unexpected error evaluating a pattern in SkyQuery: {e:?}"),
        }
    }

    /// Calculates the set of packages whose evaluation transitively depends on (e.g. via 'load'
    /// statements) the contents of the specified paths. The emitted targets are BUILD file
    /// targets.
    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1446 SkyQueryEnvironment.getRBuildFiles
    pub(crate) fn get_rbuild_files(
        &self,
        file_identifiers: &[PathFragment],
        callback: &dyn Callback<Target>,
    ) -> QueryTaskFuture<()> {
        QueryTaskFuture::from_caught(parallel_sky_query_utils::get_rbuild_files_parallel(
            self,
            file_identifiers,
            callback,
        ))
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:985 SkyQueryEnvironment.createSkyKeyUniquifier
    pub(crate) fn create_sky_key_uniquifier(&self) -> UniquifierImpl<SkyKey, SkyKey> {
        UniquifierImpl::with_parallelism(
            Arc::new(|k: &SkyKey| k.clone()),
            self.query_evaluation_parallelism_level,
        )
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:979 SkyQueryEnvironment.createMinDepthSkyKeyUniquifier
    pub(crate) fn create_min_depth_sky_key_uniquifier(
        &self,
    ) -> MinDepthUniquifierImpl<SkyKey, SkyKey> {
        MinDepthUniquifierImpl::new(
            Arc::new(|k: &SkyKey| k.clone()),
            self.query_evaluation_parallelism_level,
        )
    }

    fn get_target_internal(&self, label: &Label) -> Result<Target, GetTargetForQueryError> {
        let pkg = match self.get_package(label.get_package_identifier()) {
            Ok(pkg) => pkg,
            Err(GetPackageError::Query(e)) => return Err(GetTargetForQueryError::Query(e)),
            Err(GetPackageError::NoSuchPackage(e)) => {
                let detailed_exit_code =
                    failure_details::from_packages(&e.get_detailed_exit_code());
                return Err(TargetNotFoundException::new(Arc::new(e), detailed_exit_code).into());
            }
        };
        match pkg.get_target(label.get_name()) {
            Ok(t) => Ok(t.clone()),
            Err(e) => {
                let detailed_exit_code =
                    failure_details::from_packages(&e.get_detailed_exit_code());
                Err(TargetNotFoundException::new(Arc::new(e), detailed_exit_code).into())
            }
        }
    }
}

/// What `getPackage` throws.
enum GetPackageError {
    NoSuchPackage(NoSuchPackageException),
    Query(QueryException),
}

/// Guava `Iterables.toString`: `[a, b]` with each key's `toString`.
fn iterables_to_string(keys: &[SkyKey]) -> String {
    let parts: Vec<String> = keys.iter().map(|k| format!("{:?}", k)).collect();
    format!("[{}]", parts.join(", "))
}

// Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1304 SkyQueryEnvironment.createUnsuccessfulKeyFailure
fn create_unsuccessful_key_failure(exception: &blaze_skyframe::SkyError) -> FailureDetail {
    if let Some(detailed) = query_transitive_package_preloader::detailed_failure_detail(exception) {
        return detailed;
    }
    FailureDetail {
        message: exception.to_string(),
        category: Some(Category::Query(Query {
            code: query::Code::SkyqueryTargetException as i32,
        })),
    }
}

// Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:336 SkyQueryEnvironment.checkEvaluationResult
fn check_evaluation_result(
    universe_scope_list: &[String],
    roots: &[SkyKey],
    universe_key: &TargetPatternSequence,
    result: &EvaluationResult,
    expr_for_error: &QueryExpression,
) -> Result<(), QueryException> {
    // If the only root is the universe key, we expect to see either a single successfully
    // evaluated value or a cycle in the result or a catastrophic error.
    let values = &result.values;
    if !values.is_empty() {
        if roots.len() != 1 || roots[0] != SkyKey::new(universe_key.clone()) {
            return Ok(());
        }
        blaze_util::check_state!(
            values.len() == 1,
            "Universe query \"{:?}\" returned multiple values unexpectedly ({} values in result)",
            universe_scope_list,
            values.len()
        );
        blaze_util::check_state!(result.get(universe_key).is_some(), "{:?}", result);
        return Ok(());
    }
    blaze_util::check_state!(
        result.has_error(),
        "Universe query \"{:?}\" failed but had no error: {:?}",
        universe_scope_list,
        result
    );
    query_transitive_package_preloader::maybe_throw_query_exception_for_result_with_error(
        result,
        roots,
        expr_for_error,
        /* operation= */ "Building universe scope",
    )
}

/// Java `SkyQueryUtils.getTransitiveClosure`.
// Java: com/google/devtools/build/lib/query2/SkyQueryUtils.java:39 SkyQueryUtils.getTransitiveClosure
fn sky_query_utils_get_transitive_closure(
    targets: Vec<Target>,
    get_fwd_deps: &dyn Fn(&[Target]) -> Vec<Target>,
    visited: Arc<dyn ThreadSafeMutableSet<Target>>,
) -> Arc<dyn ThreadSafeMutableSet<Target>> {
    let mut current = targets;
    while !current.is_empty() {
        let to_visit: Vec<Target> = current
            .into_iter()
            .filter(|obj| !visited.contains(obj))
            .collect();
        current = get_fwd_deps(&to_visit);
        visited.add_all(&to_visit);
    }
    visited
}

/// Java `SkyQueryUtils.getNodesOnPath`: a depth-first search from `from` to `to`.
// Java: com/google/devtools/build/lib/query2/SkyQueryUtils.java:64 SkyQueryUtils.getNodesOnPath
fn sky_query_utils_get_nodes_on_path(
    from: &Target,
    to: &Target,
    get_fwd_deps: &dyn Fn(&[Target]) -> Vec<Target>,
) -> Option<Vec<Target>> {
    // Tree of nodes visited so far.
    let mut node_to_parent: HashMap<Label, Option<Label>> = HashMap::new();
    let mut label_to_target: HashMap<Label, Target> = HashMap::new();
    // Contains all nodes left to visit in a (LIFO) stack.
    let mut to_visit: std::collections::VecDeque<Target> = std::collections::VecDeque::new();
    to_visit.push_back(from.clone());
    node_to_parent.insert(from.get_label().clone(), None);
    label_to_target.insert(from.get_label().clone(), from.clone());
    while let Some(current) = to_visit.pop_front() {
        if to.get_label() == current.get_label() {
            // Java Digraph.getPathToTreeNode: from the root to `to`.
            let mut label_path: Vec<Label> = Vec::new();
            let mut node = Some(to.get_label().clone());
            while let Some(n) = node {
                node = node_to_parent.get(&n).cloned().flatten();
                label_path.push(n);
            }
            label_path.reverse();
            return Some(
                label_path
                    .iter()
                    .map(|item| label_to_target[item].clone())
                    .collect(),
            );
        }
        for dep in get_fwd_deps(std::slice::from_ref(&current)) {
            let dep_label = dep.get_label().clone();
            if !node_to_parent.contains_key(&dep_label) {
                node_to_parent.insert(dep_label.clone(), Some(current.get_label().clone()));
                label_to_target.insert(dep_label, dep.clone());
                to_visit.push_front(dep);
            }
        }
    }
    // Note that the only current caller of this method checks first to see if there is a path
    // before calling this method. It is not clear what the return value should be here.
    None
}

impl QueryEnvironment<Target> for SkyQueryEnvironment {
    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:991 SkyQueryEnvironment.getSiblingTargetsInPackage
    fn get_sibling_targets_in_package(&self, target: &Target) -> Result<Vec<Target>, QueryError> {
        match self
            .provider()
            .get_sibling_targets_in_package(&**self.base.event_handler(), target)
        {
            Ok(targets) => Ok(targets),
            Err(e) => Err(QueryException::with_cause(
                None,
                e.get_message(),
                Arc::new(e.clone()),
                failure_details::packages_failure_detail(&e.get_detailed_exit_code()),
            )
            .into()),
        }
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1002 SkyQueryEnvironment.getTargetsMatchingPattern
    fn get_targets_matching_pattern(
        &self,
        owner: &QueryExpression,
        pattern: &str,
        callback: &dyn Callback<Target>,
    ) -> QueryTaskFuture<()> {
        let target_pattern_key =
            match SignedTargetPattern::parse(pattern, &self.main_repo_target_parser) {
                Ok(p) => TargetPatternKey::key(p, FilteringPolicy::NoFilter),
                Err(tpe) => {
                    return match self.base.handle_error(
                        owner,
                        tpe.get_message(),
                        Some(&DetailedExitCode::of(
                            failure_details::target_parsing_failure_detail(&tpe),
                        )),
                    ) {
                        Ok(()) => self.immediate_successful_future(()),
                        Err(qe) => self.immediate_failed_future(qe),
                    };
                }
            };
        QueryTaskFuture::from_caught(self.eval_target_pattern_key(
            owner,
            &target_pattern_key,
            callback,
        ))
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1423 SkyQueryEnvironment.getOrCreate
    fn get_or_create(&self, target: &Target) -> Target {
        target.clone()
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:567 SkyQueryEnvironment.getFwdDeps
    fn get_fwd_deps(
        &self,
        targets: &[Target],
        _context: &QueryExpressionContext<Target>,
    ) -> Result<Vec<Target>, InterruptedException> {
        let mut missing_targets = Vec::new();
        let result = self.get_fwd_deps_collecting(targets, &mut missing_targets);
        if !missing_targets.is_empty() {
            self.base.event_handler().handle(&Event::warn(format!(
                "Targets were missing from graph: {}",
                iterables_to_string(&missing_targets)
            )));
        }
        Ok(result.to_vec())
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:771 SkyQueryEnvironment.getReverseDeps
    fn get_reverse_deps(
        &self,
        targets: &[Target],
        _context: &QueryExpressionContext<Target>,
    ) -> Result<Vec<Target>, InterruptedException> {
        // Java getReverseDepsOfLabels.
        let keys: Vec<SkyKey> = targets.iter().map(target_to_sky_key).collect();
        Ok(self.process_raw_reverse_deps(&self.get_raw_reverse_deps(&keys)))
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:828 SkyQueryEnvironment.getTransitiveClosure
    fn get_transitive_closure(
        &self,
        targets: &dyn ThreadSafeMutableSet<Target>,
        context: &QueryExpressionContext<Target>,
    ) -> Result<Arc<dyn ThreadSafeMutableSet<Target>>, InterruptedException> {
        Ok(sky_query_utils_get_transitive_closure(
            targets.to_vec(),
            &|targets1| self.get_fwd_deps(targets1, context).unwrap_or_default(),
            self.create_thread_safe_mutable_set(),
        ))
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1208 SkyQueryEnvironment.buildTransitiveClosure
    fn build_transitive_closure(
        &self,
        caller: &QueryExpression,
        targets: &dyn ThreadSafeMutableSet<Target>,
        _max_depth: Option<i32>,
    ) -> Result<(), QueryError> {
        // Everything has already been loaded, so here we just check for errors so that we can
        // pre-emptively throw/report if needed.
        self.report_unsuccessful_or_missing_targets_internal(&targets.to_vec(), &[], caller)
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:836 SkyQueryEnvironment.getNodesOnPath
    fn get_nodes_on_path(
        &self,
        from: &Target,
        to: &Target,
        context: &QueryExpressionContext<Target>,
    ) -> Result<Vec<Target>, InterruptedException> {
        Ok(sky_query_utils_get_nodes_on_path(from, to, &|targets| {
            self.get_fwd_deps(targets, context).unwrap_or_default()
        })
        .unwrap_or_default())
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:861 SkyQueryEnvironment.eval
    fn eval(
        &self,
        expr: &QueryExpression,
        context: &QueryExpressionContext<Target>,
        callback: &dyn Callback<Target>,
    ) -> QueryTaskFuture<()> {
        expr.eval(self, context, callback)
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:913 SkyQueryEnvironment.createEvaluateExpression
    fn create_evaluate_expression<'a>(
        &'a self,
        expr: &QueryExpression,
        context: &QueryExpressionContext<Target>,
    ) -> Box<dyn EvaluateExpression<Target> + 'a> {
        // PORT: Java's SkyQueryEvaluateExpressionImpl defers `eval` through a SettableFuture on
        // the query thread pool; with immediate futures it is the abstract implementation.
        Box::new(AbstractBlazeQueryEvaluateExpressionImpl::new(
            self, expr, context,
        ))
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:954 SkyQueryEnvironment.createThreadSafeMutableSet
    fn create_thread_safe_mutable_set(&self) -> Arc<dyn ThreadSafeMutableSet<Target>> {
        Arc::new(
            ThreadSafeMutableKeyExtractorBackedSetImpl::with_concurrency_level(
                target_key_extractor(),
                self.query_evaluation_parallelism_level,
            ),
        )
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:967 SkyQueryEnvironment.createUniquifier
    fn create_uniquifier(&self) -> Box<dyn Uniquifier<Target>> {
        Box::new(UniquifierImpl::with_parallelism(
            target_key_extractor(),
            self.query_evaluation_parallelism_level,
        ))
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:973 SkyQueryEnvironment.createMinDepthUniquifier
    fn create_min_depth_uniquifier(&self) -> Box<dyn MinDepthUniquifier<Target>> {
        Box::new(MinDepthUniquifierImpl::new(
            target_key_extractor(),
            self.query_evaluation_parallelism_level,
        ))
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:431 AbstractBlazeQueryEnvironment.handleError
    fn handle_error(
        &self,
        expression: &QueryExpression,
        message: &str,
        detailed_exit_code: Option<&DetailedExitCode>,
    ) -> Result<(), QueryException> {
        self.base
            .handle_error(expression, message, detailed_exit_code)
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1061 SkyQueryEnvironment.getTransitiveLoadFilesHelper
    fn get_transitive_load_files_helper(
        &self,
    ) -> Result<Box<dyn TransitiveLoadFilesHelper<Target> + '_>, QueryException> {
        Ok(Box::new(SkyQueryTransitiveLoadFilesHelper { env: self }))
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:511 AbstractBlazeQueryEnvironment.transitiveLoadFiles
    fn transitive_load_files(
        &self,
        targets: &[Target],
        also_add_build_files: bool,
        seen_packages: &Mutex<HashSet<PackageIdentifier>>,
        seen_bzl_labels: &Mutex<HashSet<Label>>,
        uniquifier: &dyn Uniquifier<Target>,
        helper: &dyn TransitiveLoadFilesHelper<Target>,
        callback: &dyn Callback<Target>,
    ) -> Result<(), QueryError> {
        abstract_env::transitive_load_files(
            targets,
            also_add_build_files,
            seen_packages,
            seen_bzl_labels,
            uniquifier,
            helper,
            callback,
        )
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1115 SkyQueryEnvironment.getAccessor
    fn get_accessor(&self) -> &dyn TargetAccessor<Target> {
        self
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:569 AbstractBlazeQueryEnvironment.isSettingEnabled
    fn is_setting_enabled(&self, setting: Setting) -> bool {
        self.base.is_setting_enabled(setting)
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1460 SkyQueryEnvironment.getFunctions
    fn get_functions(&self) -> Vec<Arc<dyn QueryFunction<Target>>> {
        let mut functions = self.base.get_functions();
        functions.push(Arc::new(AllRdepsFunction::new()));
        functions.push(Arc::new(RBuildFilesFunction));
        functions
    }

    fn as_streamable(&self) -> Option<&dyn StreamableQueryEnvironment<Target>> {
        Some(self)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

impl StreamableQueryEnvironment<Target> for SkyQueryEnvironment {
    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1572 SkyQueryEnvironment.getAllRdepsBoundedParallel
    fn get_all_rdeps_bounded_parallel(
        &self,
        expression: &QueryExpression,
        depth: i32,
        context: &QueryExpressionContext<Target>,
        callback: &dyn Callback<Target>,
    ) -> QueryTaskFuture<()> {
        parallel_sky_query_utils::get_all_rdeps_bounded_parallel(
            self, expression, depth, context, callback,
        )
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1563 SkyQueryEnvironment.getAllRdepsUnboundedParallel
    fn get_all_rdeps_unbounded_parallel(
        &self,
        expression: &QueryExpression,
        context: &QueryExpressionContext<Target>,
        callback: &dyn Callback<Target>,
    ) -> QueryTaskFuture<()> {
        parallel_sky_query_utils::get_all_rdeps_unbounded_parallel(
            self, expression, context, callback,
        )
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1621 SkyQueryEnvironment.getRdepsBoundedParallel
    fn get_rdeps_bounded_parallel(
        &self,
        expression: &QueryExpression,
        depth: i32,
        universe: &QueryExpression,
        context: &QueryExpressionContext<Target>,
        callback: &dyn Callback<Target>,
    ) -> QueryTaskFuture<()> {
        // Even if we need to do edge filtering, it's fine to construct the rdeps universe via an
        // unfiltered DTC visitation; the subsequent rdeps visitation will perform the edge
        // filtering.
        let universe_future =
            parallel_sky_query_utils::get_dtc_sky_key_predicate_future(self, universe, context);
        self.transform_async(universe_future, |universe_predicate| {
            parallel_sky_query_utils::get_rdeps_in_universe_bounded_parallel(
                self,
                expression,
                depth,
                &|k: &SkyKey| universe_predicate.contains(k),
                context,
                callback,
            )
        })
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1589 SkyQueryEnvironment.getRdepsUnboundedParallel
    fn get_rdeps_unbounded_parallel(
        &self,
        expression: &QueryExpression,
        universe: &QueryExpression,
        context: &QueryExpressionContext<Target>,
        callback: &dyn Callback<Target>,
    ) -> QueryTaskFuture<()> {
        let universe_future =
            parallel_sky_query_utils::get_dtc_sky_key_predicate_future(self, universe, context);
        self.transform_async(universe_future, |unfiltered_universe_predicate| {
            parallel_sky_query_utils::get_rdeps_in_universe_unbounded_parallel(
                self,
                expression,
                &|k: &SkyKey| unfiltered_universe_predicate.contains(k),
                context,
                callback,
            )
        })
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1605 SkyQueryEnvironment.getDepsUnboundedParallel
    fn get_deps_unbounded_parallel(
        &self,
        expression: &QueryExpression,
        context: &QueryExpressionContext<Target>,
        callback: &dyn Callback<Target>,
        caller: &QueryExpression,
    ) -> QueryTaskFuture<()> {
        parallel_sky_query_utils::get_deps_unbounded_parallel(
            self,
            expression,
            context,
            callback,
            /* deps_need_filtering= */ self.has_dependency_filter(),
            caller,
        )
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:617 SkyQueryEnvironment.getDepsBounded
    fn get_deps_bounded(
        &self,
        query_expression: &QueryExpression,
        context: &QueryExpressionContext<Target>,
        callback: &dyn Callback<Target>,
        depth_bound: i32,
        caller: &QueryExpression,
    ) -> QueryTaskFuture<()> {
        // Re-implement the bounded deps algorithm to allow for proper error reporting of missing
        // targets that cannot be targetified.
        let min_depth_uniquifier = self.create_min_depth_uniquifier();
        self.eval(
            query_expression,
            context,
            &blaze_query_engine::callback_fn(|partial_result: &[Target]| {
                let mut current: Vec<Target> = {
                    let set = self.create_thread_safe_mutable_set();
                    set.add_all(partial_result);
                    set.to_vec()
                };
                for i in 0..=depth_bound {
                    // Filter already visited nodes: if we see a node in a later round, then we
                    // don't need to visit it again, because the depth at which we see it at must
                    // be greater than or equal to the last visit.
                    let to_process = min_depth_uniquifier
                        .unique_at_depth_less_than_or_equal_to_all(&current, i)?;
                    callback.process(&to_process)?;

                    if i == depth_bound {
                        // We don't need to fetch dep targets any more.
                        break;
                    }

                    let mut missing_target_builder = Vec::new();
                    current = self
                        .get_fwd_deps_collecting(&to_process, &mut missing_target_builder)
                        .to_vec();
                    self.report_unsuccessful_or_missing_targets_internal(
                        &current,
                        &missing_target_builder,
                        caller,
                    )?;

                    if current.is_empty() {
                        // Exit when there are no more nodes to visit.
                        break;
                    }
                }
                Ok(())
            }),
        )
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:660 SkyQueryEnvironment.somePath
    fn some_path(
        &self,
        from_expression: &QueryExpression,
        to_expression: &QueryExpression,
        context: &QueryExpressionContext<Target>,
        callback: &dyn Callback<Target>,
        caller: &QueryExpression,
    ) -> QueryTaskFuture<()> {
        // Note that the body of this method is entirely copied from SomePathFunction, which
        // allows subclasses of SkyQueryEnvironment to override it.
        let from_value_future =
            blaze_query_engine::query_util::eval_all(self, context, from_expression);
        let to_value_future =
            blaze_query_engine::query_util::eval_all(self, context, to_expression);
        self.when_all_succeed_call(&[&from_value_future, &to_value_future], || {
            // Implementation strategy: for each x in "from", compute its forward transitive
            // closure. If it intersects "to", then do a path search from x to an arbitrary node
            // in the intersection, and return the path. This avoids computing the full
            // transitive closure of "from" in some cases.
            let from_value = from_value_future.get_if_successful();
            let to_value = to_value_future.get_if_successful();

            self.build_transitive_closure(caller, &**from_value, None)?;

            for x in from_value.to_vec() {
                let x_set = self.create_thread_safe_mutable_set();
                x_set.add(x.clone());
                let xtc = self.get_transitive_closure(&*x_set, context)?;
                // Sets.intersection(smaller-of-the-two-first... ): the first element of the
                // iteration of the set given first.
                let result: Option<Target> = if xtc.size() > to_value.size() {
                    to_value.to_vec().into_iter().find(|t| xtc.contains(t))
                } else {
                    xtc.to_vec().into_iter().find(|t| to_value.contains(t))
                };
                if let Some(first) = result {
                    callback.process(&self.get_nodes_on_path(&x, &first, context)?)?;
                    return Ok(());
                }
            }
            callback.process(&[])?;
            Ok(())
        })
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:715 SkyQueryEnvironment.allPaths
    fn all_paths(
        &self,
        from_expression: &QueryExpression,
        to_expression: &QueryExpression,
        context: &QueryExpressionContext<Target>,
        callback: &dyn Callback<Target>,
        caller: &QueryExpression,
    ) -> QueryTaskFuture<()> {
        let from_value_future =
            blaze_query_engine::query_util::eval_all(self, context, from_expression);
        let to_value_future =
            blaze_query_engine::query_util::eval_all(self, context, to_expression);
        self.when_all_succeed_call(&[&from_value_future, &to_value_future], || {
            // Algorithm: compute "reachableFromX", the forward transitive closure of the "from"
            // set, then find the intersection of "reachableFromX" with the reverse transitive
            // closure of the "to" set. The reverse transitive closure and intersection
            // operations are interleaved for efficiency. "result" holds the intersection.
            let from_value = from_value_future.get_if_successful();
            let to_value = to_value_future.get_if_successful();

            self.build_transitive_closure(caller, &**from_value, None)?;

            let reachable_from_x = self.get_transitive_closure(&**from_value, context)?;
            let uniquifier = self.create_uniquifier();
            // Java intersection(): a new HashSet. PORT: java-hash-order (HashSet<Target> by
            // Target.hashCode, identity); the elements of the iteration of the smaller set.
            let intersection: Vec<Target> = if reachable_from_x.size() > to_value.size() {
                to_value
                    .to_vec()
                    .into_iter()
                    .filter(|t| reachable_from_x.contains(t))
                    .collect()
            } else {
                reachable_from_x
                    .to_vec()
                    .into_iter()
                    .filter(|t| to_value.contains(t))
                    .collect()
            };
            let result = uniquifier.unique_all(&intersection)?;
            callback.process(&result)?;
            let mut worklist = result;
            while !worklist.is_empty() {
                let reverse_deps = self.get_reverse_deps(&worklist, context)?;
                let reachable: Vec<Target> = reverse_deps
                    .into_iter()
                    .filter(|t| reachable_from_x.contains(t))
                    .collect();
                worklist = uniquifier.unique_all(&reachable)?;
                callback.process(&worklist)?;
            }
            Ok(())
        })
    }
}

impl TargetAccessor<Target> for SkyQueryEnvironment {
    fn get_target_kind(&self, target: &Target) -> String {
        blaze_target_accessor::get_target_kind(target)
    }

    fn get_label(&self, target: &Target) -> String {
        blaze_target_accessor::get_label(target)
    }

    fn get_package(&self, target: &Target) -> String {
        blaze_target_accessor::get_package(target)
    }

    fn is_rule(&self, target: &Target) -> bool {
        blaze_target_accessor::is_rule(target)
    }

    fn is_executable_non_test_rule(&self, target: &Target) -> bool {
        blaze_target_accessor::is_executable_non_test_rule(target)
    }

    fn is_test_rule(&self, target: &Target) -> bool {
        blaze_target_accessor::is_test_rule(target)
    }

    fn is_test_suite(&self, target: &Target) -> bool {
        blaze_target_accessor::is_test_suite(target)
    }

    fn get_prerequisites(
        &self,
        caller: &QueryExpression,
        target: &Target,
        attr_name: &str,
        error_msg_prefix: &str,
    ) -> Result<Vec<Target>, QueryError> {
        blaze_target_accessor::get_prerequisites(self, caller, target, attr_name, error_msg_prefix)
    }

    fn get_string_list_attr(&self, target: &Target, attr_name: &str) -> Vec<String> {
        blaze_target_accessor::get_string_list_attr(target, attr_name)
    }

    fn get_string_attr(&self, target: &Target, attr_name: &str) -> Option<String> {
        blaze_target_accessor::get_string_attr(target, attr_name)
    }

    fn get_attr_as_string(&self, target: &Target, attr_name: &str) -> Vec<String> {
        blaze_target_accessor::get_attr_as_string(target, attr_name)
    }

    fn get_visibility<'a>(
        &'a self,
        caller: &QueryExpression,
        from: &Target,
    ) -> Result<Vec<Arc<dyn QueryVisibility<Target> + 'a>>, QueryError> {
        blaze_target_accessor::get_visibility(self, self, caller, from)
    }
}

impl AbstractBlazeQueryEnvironment for SkyQueryEnvironment {
    fn base(&self) -> &AbstractBlazeQueryEnvironmentBase {
        &self.base
    }

    fn as_query_environment(&self) -> &dyn QueryEnvironment<Target> {
        self
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:242 SkyQueryEnvironment.close
    fn close(&self) {
        // PORT: Java shuts the query thread pool down; Blaze has none.
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:462 SkyQueryEnvironment.evaluateQuery
    fn evaluate_query(
        &self,
        expr: &QueryExpression,
        callback: &dyn ThreadSafeOutputFormatterCallback<Target>,
    ) -> Result<QueryEvalResult, EvaluateQueryError> {
        match self.before_evaluate_query(expr) {
            Ok(()) => {}
            Err(QueryError::Query(e)) => return Err(e.into()),
            Err(QueryError::Interrupted(e)) => return Err(e.into()),
            Err(QueryError::Cancellation) => panic!("java.util.concurrent.CancellationException"),
        }
        // SkyQueryEnvironment batches callback invocations using a BatchStreamedCallback, created
        // here so that there's one per top-level evaluateQuery call. The batch size is large
        // enough that per-call costs of calling the original callback are amortized over a good
        // number of targets, and small enough that holding a batch of targets in memory doesn't
        // risk an OOM error.
        //
        // This flushes the batched callback prior to constructing the QueryEvalResult in the
        // unlikely case of a race between the original callback and the eventHandler.
        let batch_callback = BatchStreamedCallback {
            callback,
            uniquifier: self.create_uniquifier(),
            pending: Mutex::new(Some(Vec::new())),
            batch_threshold: BATCH_CALLBACK_SIZE,
            io_exception: IoExceptionSlot::new(),
        };
        abstract_env::evaluate_query_internal(self, expr, &batch_callback)
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:371 SkyQueryEnvironment.transformParsedQuery
    fn transform_parsed_query(&self, query_expression: QueryExpression) -> QueryExpression {
        // PORT: skipped, profiling ("transforming query") and logging (the transformed query).
        match self.get_query_expression_mapper() {
            Some(mut mapper) => query_expression.accept(&mut mapper, &()),
            // QueryExpressionMapper.identity()
            None => query_expression,
        }
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1145 SkyQueryEnvironment.getTarget
    fn get_target(&self, label: &Label) -> Result<Target, GetTargetForQueryError> {
        self.get_target_internal(label)
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1217 SkyQueryEnvironment.preloadOrThrow
    fn preload_or_throw(
        &self,
        _caller: &QueryExpression,
        _patterns: &[String],
    ) -> Result<(), PreloadError> {
        // SkyQueryEnvironment directly evaluates target patterns in #getTarget and similar
        // methods using its graph, which is prepopulated using the universeScope (see
        // #beforeEvaluateQuery), so no preloading of target patterns is necessary.
        Ok(())
    }
}

/// The environment's `TransitiveLoadFilesHelperForTargets`.
// Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1061 SkyQueryEnvironment.getTransitiveLoadFilesHelper
struct SkyQueryTransitiveLoadFilesHelper<'a> {
    env: &'a SkyQueryEnvironment,
}

impl TransitiveLoadFilesHelper<Target> for SkyQueryTransitiveLoadFilesHelper<'_> {
    fn get_pkg_id(&self, target: &Target) -> PackageIdentifier {
        abstract_env::transitive_load_files_helper_get_pkg_id(target)
    }

    fn visit_loads(
        &self,
        original_target: &Target,
        visitor: &mut dyn FnMut(&Label) -> Result<bool, QueryError>,
    ) -> Result<(), QueryError> {
        abstract_env::transitive_load_files_helper_visit_loads(original_target, visitor)
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1070 getBuildFileTarget
    fn get_build_file_target(
        &self,
        original_target: &Target,
    ) -> Result<Target, InterruptedException> {
        Ok(Target::InputFile(
            self.env.provider().get_build_file(original_target),
        ))
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1064 getLoadFileTarget
    fn get_load_file_target(
        &self,
        original_target: &Target,
        bzl_label: &Label,
    ) -> Result<Target, InterruptedException> {
        Ok(new_fake_load_target(
            bzl_label.clone(),
            &self.get_build_file_target(original_target)?,
        ))
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1075 maybeGetBuildFileTargetForLoadFileTarget
    fn maybe_get_build_file_target_for_load_file_target(
        &self,
        original_target: &Target,
        bzl_label: &Label,
    ) -> Result<Option<Target>, QueryError> {
        let package_identifier = bzl_label.get_package_identifier();
        let package_lookup_value = self
            .env
            .graph()
            .get_value(&SkyKey::new(PackageLookupKey::key(
                package_identifier.clone(),
            )))
            .and_then(|v| v.downcast::<PackageLookupValue>().ok());
        let Some(package_lookup_value) = package_lookup_value else {
            // PORT: skipped, bug reports (BugReport.sendBugReport).
            return Err(QueryException::with_failure_detail(
                None,
                format!("{} does not exist in graph", bzl_label),
                FailureDetail {
                    message: "BUILD file not found on package path".to_owned(),
                    category: Some(Category::PackageLoading(PackageLoading {
                        code: package_loading::Code::BuildFileMissing as i32,
                    })),
                },
            )
            .into());
        };
        Ok(Some(new_fake_load_target(
            Label::create_unvalidated(
                package_identifier.clone(),
                package_lookup_value
                    .get_build_file_name()
                    .get_filename_fragment()
                    .get_base_name(),
            ),
            &self.get_build_file_target(original_target)?,
        )))
    }
}

/// Wraps a callback and guarantees that all calls to the original will have at least
/// `batch_threshold` targets, except for the final such call.
// Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1498 SkyQueryEnvironment.BatchStreamedCallback
struct BatchStreamedCallback<'a> {
    callback: &'a dyn ThreadSafeOutputFormatterCallback<Target>,
    uniquifier: Box<dyn Uniquifier<Target>>,
    pending: Mutex<Option<Vec<Target>>>,
    batch_threshold: usize,
    io_exception: IoExceptionSlot,
}

impl BatchStreamedCallback<'_> {
    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1551 SkyQueryEnvironment.BatchStreamedCallback.processLastPending
    fn process_last_pending(&self) -> Result<(), OutputFormatterError> {
        let mut pending = self.pending.lock().unwrap();
        if let Some(p) = pending.as_ref()
            && !p.is_empty()
        {
            self.callback.process_output(p)?;
            *pending = None;
        }
        Ok(())
    }
}

impl Callback<Target> for BatchStreamedCallback<'_> {
    fn process(&self, partial_result: &[Target]) -> Result<(), QueryError> {
        blaze_query_engine::callback::output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<Target> for BatchStreamedCallback<'_> {
    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1521 SkyQueryEnvironment.BatchStreamedCallback.start
    fn start(&self) -> Result<(), IoException> {
        self.callback.start()
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:1526 SkyQueryEnvironment.BatchStreamedCallback.processOutput
    fn process_output(&self, partial_result: &[Target]) -> Result<(), OutputFormatterError> {
        // NonExceptionalUniquifier: unique() never throws.
        let uniquified_targets = self
            .uniquifier
            .unique_all(partial_result)
            .expect("a NonExceptionalUniquifier");
        let to_process = {
            let mut pending = self.pending.lock().unwrap();
            let p = pending
                .as_mut()
                .expect("NullPointerException: Reuse of the callback is not allowed");
            p.extend(uniquified_targets);
            if p.len() >= self.batch_threshold {
                Some(std::mem::take(p))
            } else {
                None
            }
        };
        if let Some(to_process) = to_process {
            self.callback.process_output(&to_process)?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:242 SkyQueryEnvironment.BatchStreamedCallback.close
    fn close(&self, fail_fast: bool) -> Result<(), OutputFormatterError> {
        if !fail_fast {
            self.process_last_pending()?;
        }
        self.callback.close(fail_fast)
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }
}

impl ThreadSafeOutputFormatterCallback<Target> for BatchStreamedCallback<'_> {}
