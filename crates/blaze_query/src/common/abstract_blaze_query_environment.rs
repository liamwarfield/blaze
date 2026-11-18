//! Port of `query2/common/AbstractBlazeQueryEnvironment.java`.
//!
//! PORT: Java's abstract class is split into [`AbstractBlazeQueryEnvironmentBase`] (its fields
//! and final methods), the trait [`AbstractBlazeQueryEnvironment`] (its abstract and overridable
//! methods, which concrete environments implement) and free functions taking `&dyn
//! AbstractBlazeQueryEnvironment` (its methods that call back into overridable ones, e.g.
//! `evaluateQueryInternal`). Concrete environments implement `QueryEnvironment<Target>` by
//! delegating to these. The future-composition methods come from
//! [`blaze_query_engine::QueryEnvironmentFutures`], which is the port of this class's
//! immediate-future implementations.

use std::cell::Cell;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use blaze_cmdline::{Label, PackageIdentifier};
use blaze_events::{Event, EventHandler, EventKind, ExtendedEventHandler, Postable};
use blaze_packages::attribute::dependency_filter::DependencyFilter;
use blaze_packages::exceptions::DetailedExitCode as PackagesDetailedExitCode;
use blaze_packages::target::{LabelPrinter, Target};
use blaze_protos::failure_details::{FailureDetail, Query, failure_detail::Category, query};
use blaze_query_engine::callback::{
    IoException, IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::query_util;
use blaze_query_engine::{
    Callback, DetailedExitCode, EvaluateExpression, InterruptedException, KeyExtractor,
    OutputFormatterCallback, QueryEnvironment, QueryEnvironmentFutures, QueryError,
    QueryEvalResult, QueryException, QueryExpression, QueryExpressionContext, QueryFunction,
    QueryTaskFuture, Setting, TargetNotFoundException, ThreadSafeOutputFormatterCallback,
    TransitiveLoadFilesHelper, Uniquifier, default_query_functions,
};

use crate::compat::failure_details;
use crate::pkgcache::PreloadTargetPatternsError;

/// Java `Predicate<Label> labelFilter`; `AllLabels` is `Rule.ALL_LABELS`
/// (`Predicates.alwaysTrue()`), which `validateScopeOfTargets` compares by identity.
pub enum LabelFilter {
    AllLabels,
    Predicate(Box<dyn Fn(&Label) -> bool>),
}

impl LabelFilter {
    fn test(&self, label: &Label) -> bool {
        match self {
            LabelFilter::AllLabels => true,
            LabelFilter::Predicate(p) => p(label),
        }
    }
}

/// Java `new ErrorSensingEventHandler<>(eventHandler, DetailedExitCode.class)`.
///
/// PORT: Java has one `DetailedExitCode` class; Rust events carry blaze_packages' (loading
/// errors) or blaze_query_engine's (query errors). The first error event's property of either
/// type is retained, converted to the engine's.
// Java: com/google/devtools/build/lib/events/ErrorSensingEventHandler.java:26 ErrorSensingEventHandler
pub struct QueryErrorSensingEventHandler {
    delegate: Arc<dyn ExtendedEventHandler>,
    error_property: parking_lot::Mutex<Option<DetailedExitCode>>,
    has_errors: AtomicBool,
}

impl QueryErrorSensingEventHandler {
    pub fn new(delegate: Arc<dyn ExtendedEventHandler>) -> Self {
        QueryErrorSensingEventHandler {
            delegate,
            error_property: parking_lot::Mutex::new(None),
            has_errors: AtomicBool::new(false),
        }
    }

    /// Returns whether any of the events on this objects were errors.
    pub fn has_errors(&self) -> bool {
        self.has_errors.load(Ordering::SeqCst)
    }

    /// Returns the retained error event property value, if any.
    pub fn get_error_property(&self) -> Option<DetailedExitCode> {
        self.error_property.lock().clone()
    }
}

impl EventHandler for QueryErrorSensingEventHandler {
    // Java: com/google/devtools/build/lib/events/ErrorSensingEventHandler.java:44 ErrorSensingEventHandler.handle
    fn handle(&self, e: &Event) {
        if e.get_kind() == EventKind::Error {
            self.has_errors.store(true, Ordering::SeqCst);
            let property = e.get_property::<DetailedExitCode>().cloned().or_else(|| {
                e.get_property::<PackagesDetailedExitCode>()
                    .map(failure_details::from_packages)
            });
            if let Some(property_value) = property {
                let mut slot = self.error_property.lock();
                if slot.is_none() {
                    *slot = Some(property_value);
                }
            }
        }
        self.delegate.handle(e);
    }

    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        Some(self)
    }
}

impl ExtendedEventHandler for QueryErrorSensingEventHandler {
    fn post(&self, obj: Arc<dyn Postable>) {
        self.delegate.post(obj);
    }
}

/// What `AbstractBlazeQueryEnvironment.evaluateQuery` throws.
#[derive(Debug, Clone, thiserror::Error)]
pub enum EvaluateQueryError {
    #[error(transparent)]
    Query(#[from] QueryException),
    #[error(transparent)]
    Interrupted(#[from] InterruptedException),
    #[error("{0}")]
    Io(IoException),
}

/// What `preloadOrThrow` throws: `QueryException`, `TargetParsingException` or
/// `InterruptedException`.
#[derive(Debug, Clone, thiserror::Error)]
pub enum PreloadError {
    #[error(transparent)]
    Query(#[from] QueryException),
    #[error(transparent)]
    Preload(#[from] PreloadTargetPatternsError),
}

/// What `getTarget(Label)` throws.
#[derive(Debug, Clone, thiserror::Error)]
pub enum GetTargetForQueryError {
    #[error(transparent)]
    TargetNotFound(#[from] TargetNotFoundException),
    #[error(transparent)]
    Query(#[from] QueryException),
    #[error(transparent)]
    Interrupted(#[from] InterruptedException),
}

/// The fields and final methods of Java's `AbstractBlazeQueryEnvironment<Target>`.
// Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:76 AbstractBlazeQueryEnvironment
pub struct AbstractBlazeQueryEnvironmentBase {
    pub event_handler: Arc<QueryErrorSensingEventHandler>,
    /// `event_handler` as an `Arc<dyn ExtendedEventHandler>` (what loading calls take).
    event_handler_dyn: Arc<dyn ExtendedEventHandler>,
    pub keep_going: bool,
    pub strict_scope: bool,
    pub dependency_filter: DependencyFilter,
    pub label_filter: LabelFilter,
    pub settings: HashSet<Setting>,
    pub extra_functions: Vec<Arc<dyn QueryFunction<Target>>>,
    pub label_printer: LabelPrinter,
}

impl AbstractBlazeQueryEnvironmentBase {
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:91 AbstractBlazeQueryEnvironment.<init>
    pub fn new(
        keep_going: bool,
        strict_scope: bool,
        label_filter: LabelFilter,
        event_handler: Arc<dyn ExtendedEventHandler>,
        settings: HashSet<Setting>,
        extra_functions: Vec<Arc<dyn QueryFunction<Target>>>,
        label_printer: LabelPrinter,
    ) -> Self {
        let event_handler = Arc::new(QueryErrorSensingEventHandler::new(event_handler));
        AbstractBlazeQueryEnvironmentBase {
            event_handler_dyn: event_handler.clone(),
            event_handler,
            keep_going,
            strict_scope,
            dependency_filter: Self::construct_dependency_filter(&settings),
            label_filter,
            settings,
            extra_functions,
            label_printer,
        }
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:113 AbstractBlazeQueryEnvironment.getLabelPrinter
    pub fn get_label_printer(&self) -> &LabelPrinter {
        &self.label_printer
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:117 AbstractBlazeQueryEnvironment.constructDependencyFilter
    fn construct_dependency_filter(settings: &HashSet<Setting>) -> DependencyFilter {
        let mut specified_filter = if settings.contains(&Setting::OnlyTargetDeps) {
            DependencyFilter::only_target_deps()
        } else {
            DependencyFilter::all_deps()
        };
        if settings.contains(&Setting::NoImplicitDeps) {
            specified_filter = specified_filter.and(&DependencyFilter::no_implicit_deps());
        }
        if settings.contains(&Setting::NoNodepDeps) {
            specified_filter = specified_filter.and(&DependencyFilter::no_nodep_attributes());
        }
        specified_filter
    }

    /// Java `eventHandler` as the `ExtendedEventHandler` the loading calls take.
    pub fn event_handler(&self) -> &Arc<dyn ExtendedEventHandler> {
        &self.event_handler_dyn
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:144 AbstractBlazeQueryEnvironment.createEmptyContext
    pub fn create_empty_context(&self) -> QueryExpressionContext<Target> {
        QueryExpressionContext::empty()
    }

    /// Handle an error during evaluation of `expression` by either throwing `QueryException` or
    /// emitting an event, depending on whether the evaluation is running in a "keep going" mode.
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:422 AbstractBlazeQueryEnvironment.handleError
    pub fn handle_error(
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

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:458 AbstractBlazeQueryEnvironment.validateScopeOfTargets
    pub fn validate_scope_of_targets(
        &self,
        targets: &mut Vec<Target>,
    ) -> Result<(), QueryException> {
        // Sets.filter would be more convenient here, but can't deal with exceptions.
        if !matches!(self.label_filter, LabelFilter::AllLabels) {
            // The labelFilter is always true for bazel query; it's only used for genquery rules.
            let mut kept = Vec::with_capacity(targets.len());
            for target in targets.drain(..) {
                if self.validate_scope(target.get_label(), self.strict_scope)? {
                    kept.push(target);
                }
            }
            *targets = kept;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:472 AbstractBlazeQueryEnvironment.validateScope
    pub fn validate_scope(&self, label: &Label, strict: bool) -> Result<bool, QueryException> {
        if !self.label_filter.test(label) {
            let error = format!("target '{label}' is not within the scope of the query");
            if strict {
                return Err(QueryException::new(
                    None,
                    error,
                    query::Code::TargetNotInUniverseScope,
                ));
            } else {
                self.event_handler
                    .handle(&Event::warn(format!("{error}. Skipping")));
                return Ok(false);
            }
        }
        Ok(true)
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:560 AbstractBlazeQueryEnvironment.isSettingEnabled
    pub fn is_setting_enabled(&self, setting: Setting) -> bool {
        self.settings.contains(&setting)
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:565 AbstractBlazeQueryEnvironment.getFunctions
    pub fn get_functions(&self) -> Vec<Arc<dyn QueryFunction<Target>>> {
        let mut builder = default_query_functions::<Target>();
        builder.extend(self.extra_functions.iter().cloned());
        builder
    }
}

/// A `KeyExtractor` that extracts `Label`s out of `Target`s.
// Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:573 AbstractBlazeQueryEnvironment.TargetKeyExtractor
pub fn target_key_extractor() -> Arc<dyn KeyExtractor<Target, Label>> {
    Arc::new(|element: &Target| element.get_label().clone())
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
    // else: PORT: the rate-limited logger warning is skipped (logging).
    event
}

/// The abstract and overridable methods of Java's `AbstractBlazeQueryEnvironment<Target>`.
// Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:76 AbstractBlazeQueryEnvironment
pub trait AbstractBlazeQueryEnvironment: QueryEnvironment<Target> {
    fn base(&self) -> &AbstractBlazeQueryEnvironmentBase;

    /// The environment as a `QueryEnvironment<Target>` trait object.
    fn as_query_environment(&self) -> &dyn QueryEnvironment<Target>;

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:110 AbstractBlazeQueryEnvironment.close
    fn close(&self);

    /// Used by `evaluate_query_internal` to evaluate the given `expr`. The caller is responsible
    /// for managing `callback`.
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:136 AbstractBlazeQueryEnvironment.evalTopLevelInternal
    fn eval_top_level_internal(
        &self,
        expr: &QueryExpression,
        callback: &dyn OutputFormatterCallback<Target>,
    ) -> Result<(), QueryError> {
        let env = self.as_query_environment();
        env.eval(expr, &self.base().create_empty_context(), callback)
            .get_checked()
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:148 AbstractBlazeQueryEnvironment.evaluateQuery
    fn evaluate_query(
        &self,
        expr: &QueryExpression,
        callback: &dyn ThreadSafeOutputFormatterCallback<Target>,
    ) -> Result<QueryEvalResult, EvaluateQueryError>;

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:417 AbstractBlazeQueryEnvironment.transformParsedQuery
    fn transform_parsed_query(&self, query_expression: QueryExpression) -> QueryExpression {
        query_expression
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:437 AbstractBlazeQueryEnvironment.getTarget
    fn get_target(&self, label: &Label) -> Result<Target, GetTargetForQueryError>;

    /// Perform any work that should be done ahead of time to resolve the target patterns in the
    /// query.
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:551 AbstractBlazeQueryEnvironment.preloadOrThrow
    fn preload_or_throw(
        &self,
        caller: &QueryExpression,
        patterns: &[String],
    ) -> Result<(), PreloadError>;

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:113 AbstractBlazeQueryEnvironment.getLabelPrinter
    fn get_label_printer(&self) -> &LabelPrinter {
        self.base().get_label_printer()
    }
}

/// Batch version of `getTarget(Label)`. Missing targets are absent in the returned map (in the
/// order of `labels`).
// Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:442 AbstractBlazeQueryEnvironment.getTargets
pub fn get_targets(
    env: &dyn AbstractBlazeQueryEnvironment,
    labels: &[Label],
) -> Result<Vec<(Label, Target)>, QueryError> {
    let mut result_builder: Vec<(Label, Target)> = Vec::new();
    for label in labels {
        let target = match env.get_target(label) {
            Ok(target) => target,
            // PORT: the rate-limited info log of the failure is skipped (logging).
            Err(GetTargetForQueryError::TargetNotFound(_)) => continue,
            Err(GetTargetForQueryError::Query(e)) => return Err(e.into()),
            Err(GetTargetForQueryError::Interrupted(e)) => return Err(e.into()),
        };
        // ImmutableMap.Builder.buildOrThrow: duplicate keys throw.
        if result_builder.iter().any(|(l, _)| l == label) {
            panic!("IllegalArgumentException: Multiple entries with same key: {label}");
        }
        result_builder.push((label.clone(), target));
    }
    Ok(result_builder)
}

/// A callback counting the targets it forwards.
// Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:337 AbstractBlazeQueryEnvironment.EmptinessSensingCallback
struct EmptinessSensingCallback<'a> {
    callback: &'a dyn OutputFormatterCallback<Target>,
    num_targets: AtomicI32,
    io_exception: IoExceptionSlot,
}

impl EmptinessSensingCallback<'_> {
    fn is_empty(&self) -> bool {
        self.num_targets.load(Ordering::SeqCst) == 0
    }
}

impl Callback<Target> for EmptinessSensingCallback<'_> {
    fn process(&self, partial_result: &[Target]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<Target> for EmptinessSensingCallback<'_> {
    fn start(&self) -> Result<(), IoException> {
        self.callback.start()
    }

    fn process_output(&self, partial_result: &[Target]) -> Result<(), OutputFormatterError> {
        self.num_targets
            .fetch_add(partial_result.len() as i32, Ordering::SeqCst);
        self.callback.process_output(partial_result)
    }

    fn close(&self, fail_fast: bool) -> Result<(), OutputFormatterError> {
        // PORT: the "Saw %d targets in the output" log line is skipped (logging).
        self.callback.close(fail_fast)
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }
}

/// Evaluate the specified query expression in this environment, streaming results to the given
/// `callback`. `callback.start()` will be called before query evaluation and `callback.close()`
/// will be unconditionally called at the end of query evaluation (i.e. regardless of whether it
/// was successful).
// Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:160 AbstractBlazeQueryEnvironment.evaluateQueryInternal
pub fn evaluate_query_internal(
    env: &dyn AbstractBlazeQueryEnvironment,
    expr: &QueryExpression,
    callback: &dyn ThreadSafeOutputFormatterCallback<Target>,
) -> Result<QueryEvalResult, EvaluateQueryError> {
    let base = env.base();
    let empty_sensing_callback = EmptinessSensingCallback {
        callback,
        num_targets: AtomicI32::new(0),
        io_exception: IoExceptionSlot::new(),
    };
    // In the --nokeep_going case, errors are reported in the order in which the patterns are
    // specified; using a linked hash set here makes sure that the left-most error is reported.
    let mut target_pattern_set: indexmap::IndexSet<String> = indexmap::IndexSet::new();
    expr.collect_target_patterns(&mut |p| {
        target_pattern_set.insert(p);
    });
    let patterns: Vec<String> = target_pattern_set.into_iter().collect();
    match env.preload_or_throw(expr, &patterns) {
        Ok(()) => {}
        Err(PreloadError::Query(e)) => return Err(e.into()),
        Err(PreloadError::Preload(PreloadTargetPatternsError::TargetParsing(e))) => {
            // Unfortunately, by evaluating the patterns in parallel, we lose some location
            // information.
            return Err(QueryException::with_failure_detail(
                Some(expr),
                e.to_string(),
                failure_details::target_parsing_failure_detail(&e),
            )
            .into());
        }
        Err(PreloadError::Preload(
            PreloadTargetPatternsError::TargetParsingWithFailureDetail {
                message,
                failure_detail,
            },
        )) => {
            return Err(
                QueryException::with_failure_detail(Some(expr), message, failure_detail).into(),
            );
        }
        Err(PreloadError::Preload(PreloadTargetPatternsError::Interrupted(e))) => {
            return Err(e.into());
        }
    }
    let mut io_exn: Option<IoException> = None;
    let mut fail_fast = true;
    let result = (|| -> Result<(), EvaluateQueryError> {
        empty_sensing_callback
            .start()
            .map_err(EvaluateQueryError::Io)?;
        match env.eval_top_level_internal(expr, &empty_sensing_callback) {
            Ok(()) => {}
            Err(QueryError::Query(e)) => {
                return Err(QueryException::wrap_toplevel(e, expr).into());
            }
            Err(QueryError::Interrupted(e)) => return Err(e.into()),
            Err(QueryError::Cancellation) => panic!("java.util.concurrent.CancellationException"),
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
    // PORT: the "Spent %d milliseconds evaluating query" log line is skipped (logging).

    let event_handler = &base.event_handler;
    if event_handler.has_errors() {
        let detailed_exit_code = event_handler.get_error_property();
        if !base.keep_going {
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

/// Java `AbstractBlazeQueryEnvironment.eval`: not all QueryEnvironment implementations embrace
/// the async+streaming evaluation framework. In particular, the streaming callbacks employed by
/// functions like 'deps' use `buildTransitiveClosure`. So if the implementation of that method
/// does some heavyweight blocking work, then it's best to do this blocking work in a single
/// batch. Importantly, the callback we pass in needs to maintain order.
// Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:197 AbstractBlazeQueryEnvironment.eval
pub fn abstract_eval(
    env: &dyn QueryEnvironment<Target>,
    expr: &QueryExpression,
    context: &QueryExpressionContext<Target>,
    callback: &dyn Callback<Target>,
) -> QueryTaskFuture<()> {
    let aggregate_all_callback =
        query_util::new_ordered_aggregate_all_output_formatter_callback(env);
    let eval_all_future = expr.eval(
        env,
        context,
        &blaze_query_engine::callback_fn(|partial_result: &[Target]| {
            aggregate_all_callback.process(partial_result)
        }),
    );
    env.when_succeeds_call(&eval_all_future, || {
        callback.process(&aggregate_all_callback.get_result())
    })
}

/// Wrapper for evaluating query expression in a non-streaming blaze query environment.
// Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:224 AbstractBlazeQueryEnvironment.AbstractBlazeQueryEvaluateExpressionImpl
pub struct AbstractBlazeQueryEvaluateExpressionImpl<'a> {
    env: &'a dyn QueryEnvironment<Target>,
    expression: QueryExpression,
    context: QueryExpressionContext<Target>,
    /// Whether the evaluated future was cancelled (`queryTaskFuture.isCancelled()`), or `None`
    /// before `eval`.
    query_task_future_cancelled: Cell<Option<bool>>,
}

impl<'a> AbstractBlazeQueryEvaluateExpressionImpl<'a> {
    pub fn new(
        env: &'a dyn QueryEnvironment<Target>,
        expr: &QueryExpression,
        context: &QueryExpressionContext<Target>,
    ) -> Self {
        AbstractBlazeQueryEvaluateExpressionImpl {
            env,
            expression: expr.clone(),
            context: context.clone(),
            query_task_future_cancelled: Cell::new(None),
        }
    }
}

impl EvaluateExpression<Target> for AbstractBlazeQueryEvaluateExpressionImpl<'_> {
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:236 AbstractBlazeQueryEvaluateExpressionImpl.eval
    fn eval(&self, callback: &dyn Callback<Target>) -> QueryTaskFuture<()> {
        let query_task_future = self.env.eval(&self.expression, &self.context, callback);
        self.query_task_future_cancelled
            .set(Some(query_task_future.is_cancelled()));
        query_task_future
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:244 AbstractBlazeQueryEvaluateExpressionImpl.gracefullyCancel
    fn gracefully_cancel(&self) -> bool {
        // For non-SkyQueryEnvironment-descended environments, there is no need to cancel the
        // future task, so this should be a no-op implementation.
        false
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:251 AbstractBlazeQueryEvaluateExpressionImpl.isUngracefullyCancelled
    fn is_ungracefully_cancelled(&self) -> bool {
        // Since `#gracefullyCancel` is a no-op for `AbstractBlazeQueryEvaluateExpressionImpl`
        // instance, any situation causing the `queryTaskFuture` to be cancelled should be
        // regarded as an ungraceful behavior.
        self.query_task_future_cancelled.get().unwrap_or(false)
    }
}

/// Java `TransitiveLoadFilesHelperForTargets`: `getPkgId` and `visitLoads`.
// Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:487 AbstractBlazeQueryEnvironment.TransitiveLoadFilesHelperForTargets
pub fn transitive_load_files_helper_get_pkg_id(target: &Target) -> PackageIdentifier {
    target.get_label().get_package_identifier().clone()
}

// Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:493 TransitiveLoadFilesHelperForTargets.visitLoads
pub fn transitive_load_files_helper_visit_loads(
    original_target: &Target,
    visitor: &mut dyn FnMut(&Label) -> Result<bool, QueryError>,
) -> Result<(), QueryError> {
    original_target
        .get_package_declarations()
        .visit_load_graph(visitor)
}

// Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:502 AbstractBlazeQueryEnvironment.transitiveLoadFiles
pub fn transitive_load_files(
    targets: &[Target],
    also_add_build_files: bool,
    seen_packages: &Mutex<HashSet<PackageIdentifier>>,
    seen_bzl_labels: &Mutex<HashSet<Label>>,
    uniquifier: &dyn Uniquifier<Target>,
    helper: &dyn TransitiveLoadFilesHelper<Target>,
    callback: &dyn Callback<Target>,
) -> Result<(), QueryError> {
    let mut result: Vec<Target> = Vec::new();
    for target in targets {
        let pkg_id = helper.get_pkg_id(target);
        if !seen_packages.lock().unwrap().insert(pkg_id) {
            continue;
        }

        if also_add_build_files {
            let build_file_target = helper.get_build_file_target(target)?;
            if uniquifier.unique(&build_file_target)? {
                result.push(build_file_target);
            }
        }

        helper.visit_loads(target, &mut |bzl_label: &Label| {
            if !seen_bzl_labels.lock().unwrap().insert(bzl_label.clone()) {
                return Ok(false);
            }
            let load_file_target = helper.get_load_file_target(target, bzl_label)?;
            if uniquifier.unique(&load_file_target)? {
                result.push(load_file_target);
            }
            if also_add_build_files {
                let build_file_target_for_load_file_target =
                    helper.maybe_get_build_file_target_for_load_file_target(target, bzl_label)?;
                // Can be null in genquery: see http://b/123795023#comment6.
                if let Some(t) = build_file_target_for_load_file_target {
                    if uniquifier.unique(&t)? {
                        result.push(t);
                    }
                }
            }
            Ok(true)
        })?;
    }
    callback.process(&result)
}
