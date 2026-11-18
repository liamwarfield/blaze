//! Port of `query2/query/GraphlessBlazeQueryEnvironment.java`.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use blaze_cmdline::{Label, PackageIdentifier, TargetPatternParser};
use blaze_events::ExtendedEventHandler;
use blaze_packages::attribute::Attribute;
use blaze_packages::exceptions::NoSuchThingException;
use blaze_packages::target::{LabelPrinter, Target};
use blaze_query_engine::query_util::{
    MinDepthUniquifierImpl, ThreadSafeMutableKeyExtractorBackedSetImpl, UniquifierImpl,
};
use blaze_query_engine::{
    Callback, CustomFunctionQueryEnvironment, DetailedExitCode, EvaluateExpression,
    InterruptedException, MinDepthUniquifier, QueryEnvironment, QueryEnvironmentFutures,
    QueryError, QueryEvalResult, QueryException, QueryExpression, QueryExpressionContext,
    QueryFunction, QueryTaskFuture, QueryVisibility, Setting, TargetAccessor,
    TargetNotFoundException, ThreadSafeMutableSet, ThreadSafeOutputFormatterCallback,
    TransitiveLoadFilesHelper, Uniquifier,
};
use indexmap::IndexMap;

use crate::common::abstract_blaze_query_environment::{
    self as abstract_env, AbstractBlazeQueryEnvironment, AbstractBlazeQueryEnvironmentBase,
    AbstractBlazeQueryEvaluateExpressionImpl, EvaluateQueryError, GetTargetForQueryError,
    LabelFilter, PreloadError, target_key_extractor,
};
use crate::compat::failure_details;
use crate::pkgcache::{
    CachingPackageLocator, GetSiblingsError, GetTargetError, QueryTransitivePackagePreloader,
    TargetEdgeObserver, TargetPatternPreloader, TargetProvider,
};
use crate::query::blaze_target_accessor;
use crate::query::label_visitor::LabelVisitor;
use crate::query::path_label_visitor::PathLabelVisitor;
use crate::query::target_edge_error_observer::ErrorPrintingTargetEdgeErrorObserver;

// Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:79 GraphlessBlazeQueryEnvironment.MAX_DEPTH_FULL_SCAN_LIMIT
const MAX_DEPTH_FULL_SCAN_LIMIT: i32 = 20;

/// A `QueryEnvironment` that evaluates queries without keeping a copy of the target graph; it
/// supports only `--order_output=no` (or auto with a streamed formatter).
// Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:76 GraphlessBlazeQueryEnvironment
pub struct GraphlessBlazeQueryEnvironment {
    base: AbstractBlazeQueryEnvironmentBase,
    resolved_target_patterns: RefCell<HashMap<String, Vec<Target>>>,
    target_pattern_preloader: Arc<dyn TargetPatternPreloader>,
    main_repo_target_parser: TargetPatternParser,
    query_transitive_package_preloader: Option<Arc<dyn QueryTransitivePackagePreloader>>,
    target_provider: Arc<dyn TargetProvider>,
    caching_package_locator: Arc<dyn CachingPackageLocator>,
    error_observer: ErrorPrintingTargetEdgeErrorObserver,
    label_visitor: LabelVisitor,
    loading_phase_threads: i32,
    done_query: Cell<bool>,
}

impl GraphlessBlazeQueryEnvironment {
    /// Note that the correct operation of this class critically depends on the Reporter being a
    /// singleton object, shared by all cooperating classes contributing to Query.
    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:108 GraphlessBlazeQueryEnvironment.<init>
    pub fn new(
        query_transitive_package_preloader: Option<Arc<dyn QueryTransitivePackagePreloader>>,
        target_provider: Arc<dyn TargetProvider>,
        caching_package_locator: Arc<dyn CachingPackageLocator>,
        target_pattern_preloader: Arc<dyn TargetPatternPreloader>,
        main_repo_target_parser: TargetPatternParser,
        keep_going: bool,
        strict_scope: bool,
        loading_phase_threads: i32,
        label_filter: LabelFilter,
        event_handler: Arc<dyn ExtendedEventHandler>,
        settings: HashSet<Setting>,
        extra_functions: Vec<Arc<dyn QueryFunction<Target>>>,
        label_printer: LabelPrinter,
    ) -> Self {
        let base = AbstractBlazeQueryEnvironmentBase::new(
            keep_going,
            strict_scope,
            label_filter,
            event_handler,
            settings,
            extra_functions,
            label_printer,
        );
        let error_observer = ErrorPrintingTargetEdgeErrorObserver::new(base.event_handler.clone());
        let label_visitor =
            LabelVisitor::new(target_provider.clone(), base.dependency_filter.clone());
        GraphlessBlazeQueryEnvironment {
            base,
            resolved_target_patterns: RefCell::new(HashMap::new()),
            target_pattern_preloader,
            main_repo_target_parser,
            query_transitive_package_preloader,
            target_provider,
            caching_package_locator,
            error_observer,
            label_visitor,
            loading_phase_threads,
            done_query: Cell::new(false),
        }
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:196 GraphlessBlazeQueryEnvironment.getTargetsMatchingPatternImpl
    fn get_targets_matching_pattern_impl(
        &self,
        pattern: &str,
        callback: &dyn Callback<Target>,
    ) -> Result<(), QueryError> {
        // We can safely ignore the boolean error flag. The evaluateQuery() method above wraps the
        // entire query computation in an error sensor.

        // new LinkedHashSet<>(resolvedTargetPatterns.get(pattern))
        let resolved = self
            .resolved_target_patterns
            .borrow()
            .get(pattern)
            .cloned()
            .unwrap_or_else(|| panic!("NullPointerException: pattern {pattern} not preloaded"));
        let mut targets: Vec<Target> = Vec::with_capacity(resolved.len());
        for t in resolved {
            if !targets.contains(&t) {
                targets.push(t);
            }
        }

        self.base.validate_scope_of_targets(&mut targets)?;
        callback.process(&targets)
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:442 GraphlessBlazeQueryEnvironment.preloadTransitiveClosure
    fn preload_transitive_closure(
        &self,
        targets: &[Target],
        max_depth: Option<i32>,
        caller_for_error: &QueryExpression,
    ) -> Result<(), QueryError> {
        if blaze_query_engine::query_environment::should_visit(max_depth, MAX_DEPTH_FULL_SCAN_LIMIT)
            && let Some(preloader) = &self.query_transitive_package_preloader
        {
            // Only do the full visitation if "maxDepth" is large enough. Otherwise, the
            // benefits of preloading will be outweighed by the cost of doing more work than
            // necessary.
            let mut labels: Vec<Label> = Vec::new();
            for t in targets {
                if !labels.contains(t.get_label()) {
                    labels.push(t.get_label().clone());
                }
            }
            preloader.preload_transitive_targets(
                self.base.event_handler(),
                &labels,
                self.base.keep_going,
                self.loading_phase_threads,
                if self.base.keep_going || max_depth.is_some() {
                    None
                } else {
                    Some(caller_for_error)
                },
            )?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:465 GraphlessBlazeQueryEnvironment.getTargetOrThrow
    fn get_target_or_throw(&self, label: &Label) -> Result<Target, GetTargetError> {
        // PORT: Java throws SkyframeRestartQueryException when the provider returns null; Blaze's
        // providers never do (see `TargetProvider`).
        self.target_provider
            .get_target(self.base.event_handler(), label)
    }

    /// The error check after each visitation (Java repeats it in every custom function).
    fn handle_transitive_closure_errors(&self, caller: &QueryExpression) -> Result<(), QueryError> {
        if self.error_observer.has_errors() {
            self.base.handle_error(
                caller,
                "errors were encountered while computing transitive closure",
                self.error_observer.get_detailed_exit_code().as_ref(),
            )?;
        }
        Ok(())
    }
}

/// The observer `deps` passes to its LabelVisitor.
// Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:242 GraphlessBlazeQueryEnvironment.deps
struct DepsObserver<'a> {
    error_observer: &'a ErrorPrintingTargetEdgeErrorObserver,
    result: Mutex<IndexMap<Label, Target>>,
}

impl TargetEdgeObserver for DepsObserver<'_> {
    fn edge(&self, from: &Target, attribute: Option<&Attribute>, to: &Target) {
        self.error_observer.edge(from, attribute, to);
    }

    fn missing_edge(&self, target: Option<&Target>, to: &Label, e: &NoSuchThingException) {
        self.error_observer.missing_edge(target, to, e);
    }

    fn node(&self, node: &Target) {
        self.result
            .lock()
            .unwrap()
            .entry(node.get_label().clone())
            .or_insert_with(|| node.clone());
        self.error_observer.node(node);
    }
}

impl QueryEnvironment<Target> for GraphlessBlazeQueryEnvironment {
    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:176 GraphlessBlazeQueryEnvironment.getSiblingTargetsInPackage
    fn get_sibling_targets_in_package(&self, target: &Target) -> Result<Vec<Target>, QueryError> {
        match self
            .target_provider
            .get_sibling_targets_in_package(self.base.event_handler(), target)
        {
            Ok(targets) => Ok(targets),
            Err(GetSiblingsError::NoSuchPackage(e)) => Err(QueryException::with_cause(
                None,
                e.get_message(),
                Arc::new(e.clone()),
                failure_details::packages_failure_detail(&e.get_detailed_exit_code()),
            )
            .into()),
            Err(GetSiblingsError::Interrupted(e)) => Err(e.into()),
        }
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:186 GraphlessBlazeQueryEnvironment.getTargetsMatchingPattern
    fn get_targets_matching_pattern(
        &self,
        _owner: &QueryExpression,
        pattern: &str,
        callback: &dyn Callback<Target>,
    ) -> QueryTaskFuture<()> {
        match self.get_targets_matching_pattern_impl(pattern, callback) {
            Ok(()) => self.immediate_successful_future(()),
            Err(QueryError::Query(e)) => self.immediate_failed_future(e),
            Err(QueryError::Interrupted(_)) => self.immediate_cancelled_future(),
            Err(QueryError::Cancellation) => panic!("java.util.concurrent.CancellationException"),
        }
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:218 GraphlessBlazeQueryEnvironment.getOrCreate
    fn get_or_create(&self, target: &Target) -> Target {
        target.clone()
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:223 GraphlessBlazeQueryEnvironment.getFwdDeps
    fn get_fwd_deps(
        &self,
        _targets: &[Target],
        _context: &QueryExpressionContext<Target>,
    ) -> Result<Vec<Target>, InterruptedException> {
        panic!("UnsupportedOperationException")
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:229 GraphlessBlazeQueryEnvironment.getReverseDeps
    fn get_reverse_deps(
        &self,
        _targets: &[Target],
        _context: &QueryExpressionContext<Target>,
    ) -> Result<Vec<Target>, InterruptedException> {
        panic!("UnsupportedOperationException")
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:235 GraphlessBlazeQueryEnvironment.getTransitiveClosure
    fn get_transitive_closure(
        &self,
        _targets: &dyn ThreadSafeMutableSet<Target>,
        _context: &QueryExpressionContext<Target>,
    ) -> Result<Arc<dyn ThreadSafeMutableSet<Target>>, InterruptedException> {
        panic!("UnsupportedOperationException")
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:379 GraphlessBlazeQueryEnvironment.buildTransitiveClosure
    fn build_transitive_closure(
        &self,
        caller: &QueryExpression,
        target_nodes: &dyn ThreadSafeMutableSet<Target>,
        max_depth: Option<i32>,
    ) -> Result<(), QueryError> {
        let target_nodes = target_nodes.to_vec();
        self.preload_transitive_closure(&target_nodes, max_depth, caller)?;
        self.label_visitor.sync_with_visitor(
            self.base.event_handler(),
            &target_nodes,
            self.base.keep_going,
            self.loading_phase_threads,
            max_depth,
            &self.error_observer,
        )?;
        self.handle_transitive_closure_errors(caller)
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:399 GraphlessBlazeQueryEnvironment.getNodesOnPath
    fn get_nodes_on_path(
        &self,
        _from: &Target,
        _to: &Target,
        _context: &QueryExpressionContext<Target>,
    ) -> Result<Vec<Target>, InterruptedException> {
        panic!("UnsupportedOperationException")
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:155 GraphlessBlazeQueryEnvironment.eval
    fn eval(
        &self,
        expr: &QueryExpression,
        context: &QueryExpressionContext<Target>,
        callback: &dyn Callback<Target>,
    ) -> QueryTaskFuture<()> {
        expr.eval(self, context, callback)
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:263 AbstractBlazeQueryEnvironment.createEvaluateExpression
    fn create_evaluate_expression<'a>(
        &'a self,
        expr: &QueryExpression,
        context: &QueryExpressionContext<Target>,
    ) -> Box<dyn EvaluateExpression<Target> + 'a> {
        Box::new(AbstractBlazeQueryEvaluateExpressionImpl::new(
            self, expr, context,
        ))
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:405 GraphlessBlazeQueryEnvironment.createThreadSafeMutableSet
    fn create_thread_safe_mutable_set(&self) -> Arc<dyn ThreadSafeMutableSet<Target>> {
        Arc::new(ThreadSafeMutableKeyExtractorBackedSetImpl::new(
            target_key_extractor(),
        ))
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:411 GraphlessBlazeQueryEnvironment.createUniquifier
    fn create_uniquifier(&self) -> Box<dyn Uniquifier<Target>> {
        Box::new(UniquifierImpl::new(target_key_extractor()))
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:416 GraphlessBlazeQueryEnvironment.createMinDepthUniquifier
    fn create_min_depth_uniquifier(&self) -> Box<dyn MinDepthUniquifier<Target>> {
        Box::new(MinDepthUniquifierImpl::new(
            target_key_extractor(),
            /* concurrency_level= */ 1,
        ))
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:422 AbstractBlazeQueryEnvironment.handleError
    fn handle_error(
        &self,
        expression: &QueryExpression,
        message: &str,
        detailed_exit_code: Option<&DetailedExitCode>,
    ) -> Result<(), QueryException> {
        self.base
            .handle_error(expression, message, detailed_exit_code)
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:421 GraphlessBlazeQueryEnvironment.getTransitiveLoadFilesHelper
    fn get_transitive_load_files_helper(
        &self,
    ) -> Result<Box<dyn TransitiveLoadFilesHelper<Target> + '_>, QueryException> {
        Ok(Box::new(GraphlessTransitiveLoadFilesHelper { env: self }))
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:502 AbstractBlazeQueryEnvironment.transitiveLoadFiles
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

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:477 GraphlessBlazeQueryEnvironment.getAccessor
    fn get_accessor(&self) -> &dyn TargetAccessor<Target> {
        self
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:560 AbstractBlazeQueryEnvironment.isSettingEnabled
    fn is_setting_enabled(&self, setting: Setting) -> bool {
        self.base.is_setting_enabled(setting)
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:565 AbstractBlazeQueryEnvironment.getFunctions
    fn get_functions(&self) -> Vec<Arc<dyn QueryFunction<Target>>> {
        self.base.get_functions()
    }

    fn as_custom_function_query_environment(
        &self,
    ) -> Option<&dyn CustomFunctionQueryEnvironment<Target>> {
        Some(self)
    }
}

impl CustomFunctionQueryEnvironment<Target> for GraphlessBlazeQueryEnvironment {
    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:143 GraphlessBlazeQueryEnvironment.eval
    fn eval_with_batch(
        &self,
        expr: &QueryExpression,
        context: &QueryExpressionContext<Target>,
        callback: &dyn Callback<Target>,
        batch: bool,
    ) -> QueryTaskFuture<()> {
        if batch {
            return abstract_env::abstract_eval(self, expr, context, callback);
        }
        QueryEnvironment::eval(self, expr, context, callback)
    }

    /// PORT: Java collects the nodes in `Sets.newConcurrentHashSet()` (identity-hashed targets,
    /// no deterministic order); Blaze keeps the first-visit order.
    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:242 GraphlessBlazeQueryEnvironment.deps
    fn deps(
        &self,
        from: &[Target],
        max_depth: Option<i32>,
        caller: &QueryExpression,
        callback: &dyn Callback<Target>,
    ) -> Result<(), QueryError> {
        self.preload_transitive_closure(from, max_depth, caller)?;
        let observer = DepsObserver {
            error_observer: &self.error_observer,
            result: Mutex::new(IndexMap::new()),
        };
        LabelVisitor::new(
            self.target_provider.clone(),
            self.base.dependency_filter.clone(),
        )
        .sync_uncached(
            self.base.event_handler(),
            from,
            self.base.keep_going,
            self.loading_phase_threads,
            max_depth,
            &observer,
        )?;

        self.handle_transitive_closure_errors(caller)?;
        let result: Vec<Target> = observer
            .result
            .into_inner()
            .unwrap()
            .into_values()
            .collect();
        callback.process(&result)
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:285 GraphlessBlazeQueryEnvironment.somePath
    fn some_path(
        &self,
        from: &[Target],
        to: &[Target],
        caller: &QueryExpression,
        callback: &dyn Callback<Target>,
    ) -> Result<(), QueryError> {
        self.preload_transitive_closure(from, /* max_depth= */ None, caller)?;
        let results = PathLabelVisitor::new(
            self.target_provider.clone(),
            self.base.dependency_filter.clone(),
            &self.error_observer,
        )
        .some_path(self.base.event_handler(), from, to)?;
        self.handle_transitive_closure_errors(caller)?;
        callback.process(&results)
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:303 GraphlessBlazeQueryEnvironment.allPaths
    fn all_paths(
        &self,
        from: &[Target],
        to: &[Target],
        caller: &QueryExpression,
        callback: &dyn Callback<Target>,
    ) -> Result<(), QueryError> {
        self.preload_transitive_closure(from, /* max_depth= */ None, caller)?;
        let results = PathLabelVisitor::new(
            self.target_provider.clone(),
            self.base.dependency_filter.clone(),
            &self.error_observer,
        )
        .all_paths(self.base.event_handler(), from, to)?;
        self.handle_transitive_closure_errors(caller)?;
        callback.process(&results)
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:357 GraphlessBlazeQueryEnvironment.rdeps
    fn rdeps(
        &self,
        from: &[Target],
        universe: &[Target],
        max_depth: Option<i32>,
        caller: &QueryExpression,
        callback: &dyn Callback<Target>,
    ) -> Result<(), QueryError> {
        self.preload_transitive_closure(universe, /* max_depth= */ None, caller)?;
        let results = PathLabelVisitor::new(
            self.target_provider.clone(),
            self.base.dependency_filter.clone(),
            &self.error_observer,
        )
        .rdeps(self.base.event_handler(), from, universe, max_depth)?;
        self.handle_transitive_closure_errors(caller)?;
        callback.process(&results)
    }

    /// PORT: Java preloads a `HashSet<Target>` (no deterministic order); Blaze keeps the
    /// siblings' order.
    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:321 GraphlessBlazeQueryEnvironment.samePkgDirectRdeps
    fn same_pkg_direct_rdeps(
        &self,
        from: &[Target],
        caller: &QueryExpression,
        callback: &dyn Callback<Target>,
    ) -> Result<(), QueryError> {
        let mut targets_to_preload: IndexMap<Label, Target> = IndexMap::new();
        for t in from {
            for s in self.get_sibling_targets_in_package(t)? {
                targets_to_preload.insert(s.get_label().clone(), s);
            }
        }
        let targets_to_preload: Vec<Target> = targets_to_preload.into_values().collect();
        self.preload_transitive_closure(
            &targets_to_preload,
            blaze_query_engine::same_pkg_direct_rdeps_function::DEPTH_ONE,
            caller,
        )?;
        let results = PathLabelVisitor::new(
            self.target_provider.clone(),
            self.base.dependency_filter.clone(),
            &self.error_observer,
        )
        .same_pkg_direct_rdeps(self.base.event_handler(), from)?;
        self.handle_transitive_closure_errors(caller)?;
        callback.process(&results)
    }
}

impl TargetAccessor<Target> for GraphlessBlazeQueryEnvironment {
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

impl AbstractBlazeQueryEnvironment for GraphlessBlazeQueryEnvironment {
    fn base(&self) -> &AbstractBlazeQueryEnvironmentBase {
        &self.base
    }

    fn as_query_environment(&self) -> &dyn QueryEnvironment<Target> {
        self
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:171 GraphlessBlazeQueryEnvironment.close
    fn close(&self) {
        // BlazeQueryEnvironment has no resources that need to be cleaned up.
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:162 GraphlessBlazeQueryEnvironment.evaluateQuery
    fn evaluate_query(
        &self,
        expr: &QueryExpression,
        callback: &dyn ThreadSafeOutputFormatterCallback<Target>,
    ) -> Result<QueryEvalResult, EvaluateQueryError> {
        blaze_util::check_state!(
            !self.done_query.get(),
            "Can only use environment for one query: {}",
            expr
        );
        self.done_query.set(true);
        abstract_env::evaluate_query_internal(self, expr, callback)
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:209 GraphlessBlazeQueryEnvironment.getTarget
    fn get_target(&self, label: &Label) -> Result<Target, GetTargetForQueryError> {
        match self.get_target_or_throw(label) {
            Ok(target) => Ok(target),
            Err(GetTargetError::NoSuchThing(e)) => {
                let detailed_exit_code =
                    failure_details::from_packages(&e.get_detailed_exit_code());
                Err(TargetNotFoundException::new(Arc::new(e), detailed_exit_code).into())
            }
            Err(GetTargetError::Interrupted(e)) => Err(e.into()),
        }
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:474 GraphlessBlazeQueryEnvironment.preloadOrThrow
    fn preload_or_throw(
        &self,
        _caller: &QueryExpression,
        patterns: &[String],
    ) -> Result<(), PreloadError> {
        blaze_util::check_state!(
            self.resolved_target_patterns.borrow().is_empty(),
            "Already resolved patterns: {:?} {:?}",
            patterns,
            self.resolved_target_patterns
                .borrow()
                .keys()
                .collect::<Vec<_>>()
        );
        // Note that this may throw a TargetParsingException.
        let resolved = self.target_pattern_preloader.preload_target_patterns(
            self.base.event_handler(),
            &self.main_repo_target_parser,
            patterns,
            self.base.keep_going,
        )?;
        self.resolved_target_patterns.borrow_mut().extend(resolved);
        Ok(())
    }
}

/// The environment's `TransitiveLoadFilesHelperForTargets`.
// Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:421 GraphlessBlazeQueryEnvironment.getTransitiveLoadFilesHelper
struct GraphlessTransitiveLoadFilesHelper<'a> {
    env: &'a GraphlessBlazeQueryEnvironment,
}

impl TransitiveLoadFilesHelper<Target> for GraphlessTransitiveLoadFilesHelper<'_> {
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

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:430 getBuildFileTarget
    fn get_build_file_target(
        &self,
        original_target: &Target,
    ) -> Result<Target, InterruptedException> {
        Ok(Target::InputFile(
            self.env.target_provider.get_build_file(original_target)?,
        ))
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:424 getLoadFileTarget
    fn get_load_file_target(
        &self,
        original_target: &Target,
        _bzl_label: &Label,
    ) -> Result<Target, InterruptedException> {
        let _build_file = self.get_build_file_target(original_target)?;
        // new FakeLoadTarget(bzlLabel, getBuildFileTarget(originalTarget).getPackageoid())
        blaze_util::unported!(
            "FakeLoadTarget.<init>",
            "needs a constructor from the BUILD file target's package metadata and declarations"
        )
    }

    // Java: com/google/devtools/build/lib/query2/query/GraphlessBlazeQueryEnvironment.java:436 maybeGetBuildFileTargetForLoadFileTarget
    fn maybe_get_build_file_target_for_load_file_target(
        &self,
        original_target: &Target,
        bzl_label: &Label,
    ) -> Result<Option<Target>, QueryError> {
        let pkg_id_of_bzl_label = bzl_label.get_package_identifier();
        let base_name = self
            .env
            .caching_package_locator
            .get_base_name_for_loaded_package(pkg_id_of_bzl_label);
        if base_name.is_none() {
            return Ok(None);
        }
        let _build_file = self.get_build_file_target(original_target)?;
        blaze_util::unported!(
            "FakeLoadTarget.<init>",
            "needs a constructor from the BUILD file target's package metadata and declarations"
        )
    }
}
