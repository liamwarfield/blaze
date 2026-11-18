//! Port of `query2/common/QueryTransitivePackagePreloader.java` over blaze_sky_loading's
//! `SkyframePackageManager` (TRANSITIVE_TARGET).

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_events::ExtendedEventHandler;
use blaze_packages::exceptions::{
    NoSuchPackageException, NoSuchTargetException, NoSuchThingException,
};
use blaze_protos::failure_details::{FailureDetail, Query, failure_detail::Category, query};
use blaze_query_engine::{InterruptedException, QueryError, QueryException, QueryExpression};
use blaze_sky_loading::package_manager::SkyframePackageManager;
use blaze_sky_loading::transitive_target::TransitiveTargetKey;
use blaze_skyframe::{EvaluationResult, SkyError, SkyKey};

use crate::compat::failure_details;
use crate::pkgcache::QueryTransitivePackagePreloader;

/// Preloads transitive packages for query: prepopulates Skyframe with `TransitiveTargetValue`
/// objects for the transitive closure of requested targets. To be used when doing a large
/// traversal that benefits from loading parallelism.
// Java: com/google/devtools/build/lib/query2/common/QueryTransitivePackagePreloader.java:43 QueryTransitivePackagePreloader
pub struct SkyframeQueryTransitivePackagePreloader {
    package_manager: Arc<SkyframePackageManager>,
}

impl SkyframeQueryTransitivePackagePreloader {
    pub fn new(package_manager: Arc<SkyframePackageManager>) -> Self {
        SkyframeQueryTransitivePackagePreloader { package_manager }
    }
}

/// Java `exception instanceof DetailedException`: the `FailureDetail` of the loading exceptions
/// TRANSITIVE_TARGET throws, or `None` for another exception.
fn detailed_failure_detail(exception: &SkyError) -> Option<FailureDetail> {
    let e: &(dyn std::error::Error + 'static) = &**exception;
    if let Some(e) = e.downcast_ref::<NoSuchPackageException>() {
        return Some(failure_details::packages_failure_detail(
            &e.get_detailed_exit_code(),
        ));
    }
    if let Some(e) = e.downcast_ref::<NoSuchTargetException>() {
        return Some(failure_details::packages_failure_detail(
            &e.get_detailed_exit_code(),
        ));
    }
    if let Some(e) = e.downcast_ref::<NoSuchThingException>() {
        return Some(failure_details::packages_failure_detail(
            &e.get_detailed_exit_code(),
        ));
    }
    None
}

// Java: com/google/devtools/build/lib/query2/common/QueryTransitivePackagePreloader.java:79 QueryTransitivePackagePreloader.maybeThrowQueryExceptionForResultWithError
pub fn maybe_throw_query_exception_for_result_with_error(
    result: &EvaluationResult,
    roots: &[SkyKey],
    caller: &QueryExpression,
    operation: &str,
) -> Result<(), QueryException> {
    if let Some(exception) = result.get_catastrophe() {
        return Err(throw_exception(exception, caller, operation));
    }
    let mut found_cycle = false;
    let mut exception: Option<&SkyError> = None;
    for (_, error_info) in result.error_map() {
        if !error_info.get_cycle_info().is_empty() {
            found_cycle = true;
        } else {
            exception = error_info.get_exception();
            if let Some(e) = exception
                && detailed_failure_detail(e).is_some()
            {
                break;
            }
        }
    }
    if let Some(exception) = exception {
        return Err(throw_exception(exception, caller, operation));
    }
    blaze_util::check_state!(
        found_cycle,
        "No cycle or exception found in result with error: {:?} {:?}",
        result,
        roots
    );
    Ok(())
}

// Java: com/google/devtools/build/lib/query2/common/QueryTransitivePackagePreloader.java:110 QueryTransitivePackagePreloader.throwException
fn throw_exception(
    exception: &SkyError,
    caller: &QueryExpression,
    operation: &str,
) -> QueryException {
    let failure_detail = match detailed_failure_detail(exception) {
        Some(failure_detail) => failure_detail,
        None => {
            // PORT: bugReporter.sendNonFatalBugReport skipped (bug reports).
            FailureDetail {
                message: String::new(),
                category: Some(Category::Query(Query {
                    code: query::Code::NonDetailedError as i32,
                })),
            }
        }
    };
    QueryException::with_cause(
        Some(caller),
        format!("{operation} failed: {exception}"),
        exception.clone(),
        failure_detail,
    )
}

impl QueryTransitivePackagePreloader for SkyframeQueryTransitivePackagePreloader {
    /// Unconditionally evaluates the transitive closure of the given labels; on errors throws a
    /// `QueryException` attributed to `caller_for_error` when it is set.
    // Java: com/google/devtools/build/lib/query2/common/QueryTransitivePackagePreloader.java:137 QueryTransitivePackagePreloader.preloadTransitiveTargets
    fn preload_transitive_targets(
        &self,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        labels_to_visit: &[Label],
        keep_going: bool,
        parallel_threads: i32,
        caller_for_error: Option<&QueryExpression>,
    ) -> Result<(), QueryError> {
        let value_names: Vec<SkyKey> = labels_to_visit
            .iter()
            .map(|label| SkyKey::new(TransitiveTargetKey::of(label.clone())))
            .collect();
        // PORT: Java's EvaluationContext sets detectCycles(false); the blocking evaluator
        // reports what it finds.
        let result = self
            .package_manager
            .evaluate(
                &value_names,
                keep_going,
                parallel_threads.max(1) as usize,
                event_handler.clone(),
            )
            .map_err(|_| QueryError::Interrupted(InterruptedException::new()))?;
        if !result.has_error() {
            return Ok(());
        }
        if let Some(caller_for_error) = caller_for_error {
            maybe_throw_query_exception_for_result_with_error(
                &result,
                &value_names,
                caller_for_error,
                "preloading transitive closure",
            )?;
            return Ok(());
        }
        if keep_going && result.get_catastrophe().is_none() {
            return Ok(());
        }
        // At this point, we are not in keep going mode, and we have no caller to throw
        // exceptions to. Java runs an empty evaluation so the graph is not left in an
        // inconsistent state; PORT: skipped, incremental.
        Ok(())
    }
}
