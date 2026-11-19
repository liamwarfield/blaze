//! Port of `query2/query/BlazeQueryEnvironment.java`: the query environment that keeps the
//! target graph it visits (a `Digraph<Target>`), used for ordered (`--order_output=full|deps`)
//! and graph-shaped (`graph`, `minrank`, `maxrank`) output.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use blaze_cmdline::{Label, PackageIdentifier, TargetPatternParser};
use blaze_events::ExtendedEventHandler;
use blaze_packages::attribute::Attribute;
use blaze_packages::exceptions::NoSuchThingException;
use blaze_packages::target::{FakeLoadTarget, LabelPrinter, Target};
use blaze_query_engine::query_util::{
    MinDepthUniquifierImpl, ThreadSafeMutableKeyExtractorBackedSetImpl, UniquifierImpl,
};
use blaze_query_engine::{
    Callback, DetailedExitCode, EvaluateExpression, InterruptedException, MinDepthUniquifier,
    QueryEnvironment, QueryEnvironmentFutures, QueryError, QueryEvalResult, QueryException,
    QueryExpression, QueryExpressionContext, QueryFunction, QueryTaskFuture, QueryVisibility,
    Setting, TargetAccessor, TargetNotFoundException, ThreadSafeMutableSet,
    ThreadSafeOutputFormatterCallback, TransitiveLoadFilesHelper, Uniquifier,
};
use blaze_util::graph::{Digraph, NodeId};

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
use crate::query::target_edge_error_observer::ErrorPrintingTargetEdgeErrorObserver;

// Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:78 BlazeQueryEnvironment.MAX_DEPTH_FULL_SCAN_LIMIT
const MAX_DEPTH_FULL_SCAN_LIMIT: i32 = 20;

/// A `Target` as the label of a `Digraph` node (Java `Digraph<Target>`).
///
/// PORT: Java targets are hash keys by identity; Blaze's `Target` has no `Hash`, so this
/// wrapper hashes the label and compares with `Target`'s equality (identity, as in Java). The
/// hash only places nodes in the graph's index; the node order is the creation order (see
/// `blaze_util::graph::Digraph`).
#[derive(Clone)]
pub struct TargetNode(pub Target);

impl PartialEq for TargetNode {
    fn eq(&self, other: &TargetNode) -> bool {
        self.0 == other.0
    }
}

impl Eq for TargetNode {}

impl Hash for TargetNode {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.get_label().hash(state);
    }
}

impl fmt::Debug for TargetNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.get_label())
    }
}

/// Java `Digraph<Target>`.
pub type TargetDigraph = Digraph<TargetNode>;

/// The standard `QueryEnvironment` for Blaze.
// Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:77 BlazeQueryEnvironment
pub struct BlazeQueryEnvironment {
    base: AbstractBlazeQueryEnvironmentBase,
    resolved_target_patterns: RefCell<HashMap<String, Vec<Target>>>,
    target_pattern_preloader: Arc<dyn TargetPatternPreloader>,
    target_parser: TargetPatternParser,
    query_transitive_package_preloader: Option<Arc<dyn QueryTransitivePackagePreloader>>,
    target_provider: Arc<dyn TargetProvider>,
    caching_package_locator: Arc<dyn CachingPackageLocator>,
    graph: Rc<RefCell<TargetDigraph>>,
    error_observer: ErrorPrintingTargetEdgeErrorObserver,
    label_visitor: LabelVisitor,
    loading_phase_threads: i32,
    done_query: Cell<bool>,
}

impl BlazeQueryEnvironment {
    /// Note that the correct operation of this class critically depends on the Reporter being a
    /// singleton object, shared by all cooperating classes contributing to Query.
    ///
    /// `strict_scope`: if true, fail the whole query if a label goes out of scope.
    /// `loading_phase_threads`: the number of threads to use during loading the packages for the
    /// query. `label_filter`: a predicate that determines if a specific label is allowed to be
    /// visited during query execution. If it returns false, the query execution is stopped with
    /// an error message. `settings`: a set of enabled settings.
    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:106 BlazeQueryEnvironment.<init>
    pub fn new(
        query_transitive_package_preloader: Option<Arc<dyn QueryTransitivePackagePreloader>>,
        target_provider: Arc<dyn TargetProvider>,
        caching_package_locator: Arc<dyn CachingPackageLocator>,
        target_pattern_preloader: Arc<dyn TargetPatternPreloader>,
        target_parser: TargetPatternParser,
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
        BlazeQueryEnvironment {
            base,
            resolved_target_patterns: RefCell::new(HashMap::new()),
            target_pattern_preloader,
            target_parser,
            query_transitive_package_preloader,
            target_provider,
            caching_package_locator,
            graph: Rc::new(RefCell::new(Digraph::new())),
            error_observer,
            label_visitor,
            loading_phase_threads,
            done_query: Cell::new(false),
        }
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:178 BlazeQueryEnvironment.getTargetsMatchingPatternImpl
    fn get_targets_matching_pattern_impl(
        &self,
        pattern: &str,
        callback: &dyn Callback<Target>,
    ) -> Result<(), QueryError> {
        // We can safely ignore the boolean error flag. The evaluateQuery() method above wraps the
        // entire query computation in an error sensor.

        // This must be a collections class with a fast contains() implementation, or the code
        // below becomes quadratic in runtime.
        let resolved = self
            .resolved_target_patterns
            .borrow()
            .get(pattern)
            .cloned()
            .unwrap_or_else(|| panic!("NullPointerException: pattern {pattern} not preloaded"));
        let mut targets: Vec<Target> = Vec::with_capacity(resolved.len());
        let mut target_set: HashSet<TargetNode> = HashSet::with_capacity(resolved.len());
        for t in resolved {
            if target_set.insert(TargetNode(t.clone())) {
                targets.push(t);
            }
        }

        self.base.validate_scope_of_targets(&mut targets)?;
        if targets.len() != target_set.len() {
            target_set = targets.iter().map(|t| TargetNode(t.clone())).collect();
        }

        let mut packages: HashSet<PackageIdentifier> = HashSet::new();
        for target in &targets {
            packages.insert(target.get_label().get_package_identifier().clone());
        }

        for target in &targets {
            // This triggers node creation in the Digraph; getOrCreate(X) returns X.
            self.get_or_create(target);

            // Preservation of graph order: it is important that targets obtained via a wildcard
            // such as p:* are correctly ordered w.r.t. each other, so to ensure this, we add
            // edges between any pair of directly connected targets in this set.
            if let Some(output_file) = target.as_output_file() {
                let generating_rule = Target::Rule(output_file.get_generating_rule().clone());
                if target_set.contains(&TargetNode(generating_rule.clone())) {
                    self.make_edge(target, &generating_rule);
                }
            } else if let Some(rule) = target.as_rule() {
                for label in rule.get_sorted_labels(&self.base.dependency_filter) {
                    if !packages.contains(label.get_package_identifier()) {
                        continue; // don't cause additional package loading
                    }
                    if !self.base.validate_scope(&label, self.base.strict_scope)? {
                        continue; // Don't create edges to targets which are out of scope.
                    }
                    match self.get_target_or_throw(&label) {
                        Ok(to) => {
                            if target_set.contains(&TargetNode(to.clone())) {
                                self.make_edge(target, &to);
                            }
                        }
                        Err(GetTargetError::NoSuchThing(_)) => { /* ignore */ }
                        Err(GetTargetError::Interrupted(e)) => return Err(e.into()),
                    }
                }
            }
        }
        callback.process(&targets)
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:240 BlazeQueryEnvironment.getNode
    fn get_node(&self, target: &Target) -> NodeId {
        self.graph
            .borrow_mut()
            .create_node(TargetNode(target.clone()))
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:244 BlazeQueryEnvironment.getNodes
    fn get_nodes(&self, targets: &[Target]) -> Vec<NodeId> {
        let mut result: Vec<NodeId> = Vec::with_capacity(targets.len());
        for t in targets {
            let node = self.get_node(t);
            if !result.contains(&node) {
                result.push(node);
            }
        }
        result
    }

    /// The target of a node of the graph (Java `node.getLabel()`).
    fn node_target(&self, node: NodeId) -> Target {
        self.graph.borrow().get_label(node).0.clone()
    }

    /// Checks that the graph rooted at 'targetNode' has been completely built; fails if not.
    /// Callers of `get_transitive_closure` must ensure that `build_transitive_closure` has been
    /// called before.
    ///
    /// It would be inefficient and failure-prone to make getTransitiveClosure call
    /// buildTransitiveClosure directly. Also, it would cause nondeterministic behavior of the
    /// operators, since the set of packages loaded (and hence errors reported) would depend on the
    /// ordering details of the query operators' implementations.
    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:296 BlazeQueryEnvironment.checkBuilt
    fn check_built(&self, target_node: &Target) {
        blaze_util::check_state!(
            self.label_visitor.has_visited(target_node.get_label()),
            "getTransitiveClosure({}) called without prior call to buildTransitiveClosure()",
            target_node
        );
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:389 BlazeQueryEnvironment.preloadTransitiveClosure
    fn preload_transitive_closure(
        &self,
        targets: &[Target],
        max_depth: Option<i32>,
        caller_for_error: &QueryExpression,
    ) -> Result<(), QueryError> {
        if blaze_query_engine::query_environment::should_visit(max_depth, MAX_DEPTH_FULL_SCAN_LIMIT)
            && let Some(preloader) = &self.query_transitive_package_preloader
        {
            // Only do the full visitation if "maxDepth" is large enough. Otherwise, the benefits
            // of preloading will be outweighed by the cost of doing more work than necessary.
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
                // Don't throw an error if in keep-going mode or if the depth was limited: it's
                // possible that an encountered error was deeper than the depth bound.
                if self.base.keep_going || max_depth.is_some() {
                    None
                } else {
                    Some(caller_for_error)
                },
            )?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:438 BlazeQueryEnvironment.makeEdge
    fn make_edge(&self, from: &Target, to: &Target) {
        self.graph
            .borrow_mut()
            .add_edge(TargetNode(from.clone()), TargetNode(to.clone()));
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:442 BlazeQueryEnvironment.getTargetOrThrow
    fn get_target_or_throw(&self, label: &Label) -> Result<Target, GetTargetError> {
        // PORT: Java throws SkyframeRestartQueryException when the provider returns null; Blaze's
        // providers never do (see `TargetProvider`).
        self.target_provider
            .get_target(self.base.event_handler(), label)
    }

    /// Given a set of target nodes, returns the targets.
    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:474 BlazeQueryEnvironment.getTargetsFromNodes
    fn get_targets_from_nodes(&self, input: &[NodeId]) -> Arc<dyn ThreadSafeMutableSet<Target>> {
        let result = self.create_thread_safe_mutable_set();
        for node in input {
            result.add(self.node_target(*node));
        }
        result
    }

    /// The environment's graph (Java: the `Digraph` of the `DigraphQueryEvalResult`).
    pub fn get_graph(&self) -> Rc<RefCell<TargetDigraph>> {
        self.graph.clone()
    }
}

/// It suffices to synchronize the modifications of this.graph from within the
/// GraphBuildingObserver, because that's the only concurrent part. Concurrency is always
/// encapsulated within the evaluation of a single query operator (e.g. deps(), somepath(), etc).
// Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:413 BlazeQueryEnvironment.GraphBuildingObserver
struct GraphBuildingObserver<'a> {
    env: &'a BlazeQueryEnvironment,
}

impl TargetEdgeObserver for GraphBuildingObserver<'_> {
    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:416 BlazeQueryEnvironment.GraphBuildingObserver.edge
    fn edge(&self, from: &Target, attribute: Option<&Attribute>, to: &Target) {
        blaze_util::check_state!(
            match attribute {
                None => true,
                Some(attribute) => {
                    let rule = from.as_rule().expect("ClassCastException: not a Rule");
                    self.env.base.dependency_filter.test(&**rule, attribute)
                }
            },
            "Disallowed edge from LabelVisitor: {} --> {}",
            from,
            to
        );
        self.env.make_edge(from, to);
        self.env.error_observer.edge(from, attribute, to);
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:427 BlazeQueryEnvironment.GraphBuildingObserver.node
    fn node(&self, node: &Target) {
        self.env
            .graph
            .borrow_mut()
            .create_node(TargetNode(node.clone()));
        self.env.error_observer.node(node);
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:433 BlazeQueryEnvironment.GraphBuildingObserver.missingEdge
    fn missing_edge(&self, target: Option<&Target>, to: &Label, e: &NoSuchThingException) {
        self.env.error_observer.missing_edge(target, to, e);
    }
}

impl QueryEnvironment<Target> for BlazeQueryEnvironment {
    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:152 BlazeQueryEnvironment.getSiblingTargetsInPackage
    fn get_sibling_targets_in_package(&self, target: &Target) -> Result<Vec<Target>, QueryError> {
        let siblings = match self
            .target_provider
            .get_sibling_targets_in_package(self.base.event_handler(), target)
        {
            Ok(targets) => targets,
            Err(GetSiblingsError::NoSuchPackage(e)) => {
                return Err(QueryException::with_cause(
                    None,
                    e.get_message(),
                    Arc::new(e.clone()),
                    failure_details::packages_failure_detail(&e.get_detailed_exit_code()),
                )
                .into());
            }
            Err(GetSiblingsError::Interrupted(e)) => return Err(e.into()),
        };
        // Ensure that the sibling targets are in the graph being built-up.
        for sibling in &siblings {
            self.get_node(sibling);
        }
        Ok(siblings)
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:166 BlazeQueryEnvironment.getTargetsMatchingPattern
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

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:253 BlazeQueryEnvironment.getOrCreate
    fn get_or_create(&self, target: &Target) -> Target {
        let node = self.get_node(target);
        self.node_target(node)
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:258 BlazeQueryEnvironment.getFwdDeps
    fn get_fwd_deps(
        &self,
        targets: &[Target],
        _context: &QueryExpressionContext<Target>,
    ) -> Result<Vec<Target>, InterruptedException> {
        let result = self.create_thread_safe_mutable_set();
        for target in targets {
            let node = self.get_node(target);
            let successors = self.graph.borrow().get_successors(node).to_vec();
            result.add_all(&self.get_targets_from_nodes(&successors).to_vec());
        }
        Ok(result.to_vec())
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:268 BlazeQueryEnvironment.getReverseDeps
    fn get_reverse_deps(
        &self,
        targets: &[Target],
        _context: &QueryExpressionContext<Target>,
    ) -> Result<Vec<Target>, InterruptedException> {
        let result = self.create_thread_safe_mutable_set();
        for target in targets {
            let node = self.get_node(target);
            let predecessors = self.graph.borrow().get_predecessors(node).to_vec();
            result.add_all(&self.get_targets_from_nodes(&predecessors).to_vec());
        }
        Ok(result.to_vec())
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:278 BlazeQueryEnvironment.getTransitiveClosure
    fn get_transitive_closure(
        &self,
        target_nodes: &dyn ThreadSafeMutableSet<Target>,
        _context: &QueryExpressionContext<Target>,
    ) -> Result<Arc<dyn ThreadSafeMutableSet<Target>>, InterruptedException> {
        let targets = target_nodes.to_vec();
        for node in &targets {
            self.check_built(node);
        }
        let start_nodes = self.get_nodes(&targets);
        let reachable = self.graph.borrow().get_fwd_reachable(&start_nodes);
        Ok(self.get_targets_from_nodes(&reachable))
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:304 BlazeQueryEnvironment.buildTransitiveClosure
    fn build_transitive_closure(
        &self,
        caller: &QueryExpression,
        target_nodes: &dyn ThreadSafeMutableSet<Target>,
        max_depth: Option<i32>,
    ) -> Result<(), QueryError> {
        let target_nodes = target_nodes.to_vec();
        // PORT: Profiler "preloadTransitiveClosure" / "syncWithVisitor" skipped (profiling).
        self.preload_transitive_closure(&target_nodes, max_depth, caller)?;
        self.label_visitor.sync_with_visitor(
            self.base.event_handler(),
            &target_nodes,
            self.base.keep_going,
            self.loading_phase_threads,
            max_depth,
            &GraphBuildingObserver { env: self },
        )?;

        if self.error_observer.has_errors() {
            self.base.handle_error(
                caller,
                "errors were encountered while computing transitive closure",
                self.error_observer.get_detailed_exit_code().as_ref(),
            )?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:329 BlazeQueryEnvironment.getNodesOnPath
    fn get_nodes_on_path(
        &self,
        from: &Target,
        to: &Target,
        _context: &QueryExpressionContext<Target>,
    ) -> Result<Vec<Target>, InterruptedException> {
        let from_node = self.get_node(from);
        let to_node = self.get_node(to);
        let path = self
            .graph
            .borrow()
            .get_shortest_path(from_node, to_node)
            .unwrap_or_else(|| panic!("NullPointerException: no path from {from} to {to}"));
        Ok(path.into_iter().map(|n| self.node_target(n)).collect())
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:197 AbstractBlazeQueryEnvironment.eval
    fn eval(
        &self,
        expr: &QueryExpression,
        context: &QueryExpressionContext<Target>,
        callback: &dyn Callback<Target>,
    ) -> QueryTaskFuture<()> {
        abstract_env::abstract_eval(self, expr, context, callback)
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

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:340 BlazeQueryEnvironment.createThreadSafeMutableSet
    fn create_thread_safe_mutable_set(&self) -> Arc<dyn ThreadSafeMutableSet<Target>> {
        Arc::new(ThreadSafeMutableKeyExtractorBackedSetImpl::new(
            target_key_extractor(),
        ))
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:346 BlazeQueryEnvironment.createUniquifier
    fn create_uniquifier(&self) -> Box<dyn Uniquifier<Target>> {
        Box::new(UniquifierImpl::new(target_key_extractor()))
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:351 BlazeQueryEnvironment.createMinDepthUniquifier
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

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:356 BlazeQueryEnvironment.getTransitiveLoadFilesHelper
    fn get_transitive_load_files_helper(
        &self,
    ) -> Result<Box<dyn TransitiveLoadFilesHelper<Target> + '_>, QueryException> {
        Ok(Box::new(BlazeTransitiveLoadFilesHelper { env: self }))
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

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:469 BlazeQueryEnvironment.getAccessor
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
}

/// Java `BlazeTargetAccessor`, which every method of this environment delegates to.
impl TargetAccessor<Target> for BlazeQueryEnvironment {
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

impl AbstractBlazeQueryEnvironment for BlazeQueryEnvironment {
    fn base(&self) -> &AbstractBlazeQueryEnvironmentBase {
        &self.base
    }

    fn as_query_environment(&self) -> &dyn QueryEnvironment<Target> {
        self
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:133 BlazeQueryEnvironment.close
    fn close(&self) {
        // BlazeQueryEnvironment has no resources that need to be cleaned up.
    }

    /// PORT: Java returns a `DigraphQueryEvalResult` holding `graph`; Blaze returns the
    /// `QueryEvalResult` part and hands out the graph through [`Self::get_result_graph`].
    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:138 BlazeQueryEnvironment.evaluateQuery
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

    fn get_result_graph(&self) -> Option<Rc<RefCell<TargetDigraph>>> {
        Some(self.graph.clone())
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:229 BlazeQueryEnvironment.getTarget
    fn get_target(&self, label: &Label) -> Result<Target, GetTargetForQueryError> {
        // Can't use strictScope here because we are expecting a target back.
        self.base.validate_scope(label, true)?;
        match self.get_target_or_throw(label) {
            Ok(target) => {
                let node = self.get_node(&target);
                Ok(self.node_target(node))
            }
            Err(GetTargetError::NoSuchThing(e)) => {
                let detailed_exit_code =
                    failure_details::from_packages(&e.get_detailed_exit_code());
                Err(TargetNotFoundException::new(Arc::new(e), detailed_exit_code).into())
            }
            Err(GetTargetError::Interrupted(e)) => Err(e.into()),
        }
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:454 BlazeQueryEnvironment.preloadOrThrow
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
        // Note that this may throw a RuntimeException if deps are missing in Skyframe and this is
        // being called from within a SkyFunction.
        let resolved = self.target_pattern_preloader.preload_target_patterns(
            self.base.event_handler(),
            &self.target_parser,
            patterns,
            self.base.keep_going,
        )?;
        self.resolved_target_patterns.borrow_mut().extend(resolved);
        Ok(())
    }
}

/// The environment's `TransitiveLoadFilesHelperForTargets`.
// Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:356 BlazeQueryEnvironment.getTransitiveLoadFilesHelper
struct BlazeTransitiveLoadFilesHelper<'a> {
    env: &'a BlazeQueryEnvironment,
}

impl TransitiveLoadFilesHelper<Target> for BlazeTransitiveLoadFilesHelper<'_> {
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

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:367 getBuildFileTarget
    fn get_build_file_target(
        &self,
        original_target: &Target,
    ) -> Result<Target, InterruptedException> {
        Ok(Target::InputFile(
            self.env.target_provider.get_build_file(original_target)?,
        ))
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:359 getLoadFileTarget
    fn get_load_file_target(
        &self,
        original_target: &Target,
        bzl_label: &Label,
    ) -> Result<Target, InterruptedException> {
        let build_file = self.get_build_file_target(original_target)?;
        let fake = new_fake_load_target(bzl_label.clone(), &build_file);
        let node = self.env.get_node(&fake);
        Ok(self.env.node_target(node))
    }

    // Java: com/google/devtools/build/lib/query2/query/BlazeQueryEnvironment.java:373 maybeGetBuildFileTargetForLoadFileTarget
    fn maybe_get_build_file_target_for_load_file_target(
        &self,
        original_target: &Target,
        bzl_label: &Label,
    ) -> Result<Option<Target>, QueryError> {
        let pkg_id_of_bzl_label = bzl_label.get_package_identifier();
        let Some(base_name) = self
            .env
            .caching_package_locator
            .get_base_name_for_loaded_package(pkg_id_of_bzl_label)
        else {
            return Ok(None);
        };
        let build_file = self.get_build_file_target(original_target)?;
        let fake = new_fake_load_target(
            Label::create_unvalidated(pkg_id_of_bzl_label.clone(), &base_name),
            &build_file,
        );
        let node = self.env.get_node(&fake);
        Ok(Some(self.env.node_target(node)))
    }
}

/// `new FakeLoadTarget(label, buildFileTarget.getPackageoid())`.
pub(crate) fn new_fake_load_target(label: Label, build_file_target: &Target) -> Target {
    let build_file = build_file_target
        .as_input_file()
        .expect("the BUILD file target is an InputFile");
    Target::FakeLoadTarget(Arc::new(FakeLoadTarget::from_parts(
        label,
        build_file.get_package_metadata().clone(),
        build_file.get_package_declarations().clone(),
    )))
}
