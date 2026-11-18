//! Ports of `query2/query/TargetEdgeErrorObserver.java` and
//! `ErrorPrintingTargetEdgeErrorObserver.java`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use blaze_cmdline::Label;
use blaze_events::{Event, EventHandler};
use blaze_packages::attribute::Attribute;
use blaze_packages::exceptions::NoSuchThingException;
use blaze_packages::target::Target;
use blaze_packages::target::target_utils;
use blaze_query_engine::DetailedExitCode;

use crate::compat::failure_details;
use crate::pkgcache::TargetEdgeObserver;

/// Record errors, such as missing package/target or rules containing errors, encountered during
/// visitation. Emit an error message upon encountering missing edges.
///
/// If you need to report errors to the console during visitation, use the subclass
/// [`ErrorPrintingTargetEdgeErrorObserver`].
// Java: com/google/devtools/build/lib/query2/query/TargetEdgeErrorObserver.java:40 TargetEdgeErrorObserver
#[derive(Default)]
pub struct TargetEdgeErrorObserver {
    has_errors: AtomicBool,
    error_code: parking_lot::Mutex<Option<DetailedExitCode>>,
}

impl TargetEdgeErrorObserver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reports an unresolved label error and records the fact that an error was encountered.
    // Java: com/google/devtools/build/lib/query2/query/TargetEdgeErrorObserver.java:60 TargetEdgeErrorObserver.missingEdge
    fn missing_edge_impl(&self, e: &NoSuchThingException) {
        self.has_errors.store(true, Ordering::SeqCst);
        let mut error_code = self.error_code.lock();
        if error_code.is_none() {
            *error_code = Some(failure_details::from_packages(&e.get_detailed_exit_code()));
        }
    }

    /// Returns true iff any errors (such as missing targets or packages, or rules in error) were
    /// encountered during any work.
    // Java: com/google/devtools/build/lib/query2/query/TargetEdgeErrorObserver.java:74 TargetEdgeErrorObserver.hasErrors
    pub fn has_errors(&self) -> bool {
        self.has_errors.load(Ordering::SeqCst)
    }

    // Java: com/google/devtools/build/lib/query2/query/TargetEdgeErrorObserver.java:80 TargetEdgeErrorObserver.getDetailedExitCode
    pub fn get_detailed_exit_code(&self) -> Option<DetailedExitCode> {
        self.error_code.lock().clone()
    }

    // Java: com/google/devtools/build/lib/query2/query/TargetEdgeErrorObserver.java:90 TargetEdgeErrorObserver.node
    fn node_impl(&self, node: &Target) {
        let declarations = node.get_package_declarations();
        let rule_contains_errors = match node {
            Target::Rule(rule) => rule.contains_errors(),
            _ => false,
        };
        if declarations.contains_errors() || rule_contains_errors {
            self.has_errors.store(true, Ordering::SeqCst);
            match declarations.get_failure_detail() {
                Some(failure_detail) => {
                    let mut error_code = self.error_code.lock();
                    if error_code.is_none() {
                        *error_code = Some(failure_details::from_packages(&failure_detail));
                    }
                }
                None => {
                    // PORT: BugReport.sendNonFatalBugReport("Undetailed error from package: ...")
                    // skipped (bug reports).
                }
            }
        }
    }
}

impl TargetEdgeObserver for TargetEdgeErrorObserver {
    fn edge(&self, _from: &Target, _attribute: Option<&Attribute>, _to: &Target) {}

    fn missing_edge(&self, _target: Option<&Target>, _to: &Label, e: &NoSuchThingException) {
        self.missing_edge_impl(e);
    }

    fn node(&self, node: &Target) {
        self.node_impl(node);
    }
}

/// Record errors, such as missing package/target or rules containing errors, encountered during
/// visitation. Emit an error message upon encountering missing edges.
// Java: com/google/devtools/build/lib/query2/query/ErrorPrintingTargetEdgeErrorObserver.java:35 ErrorPrintingTargetEdgeErrorObserver
pub struct ErrorPrintingTargetEdgeErrorObserver {
    super_: TargetEdgeErrorObserver,
    event_handler: Arc<dyn EventHandler>,
}

impl ErrorPrintingTargetEdgeErrorObserver {
    // Java: com/google/devtools/build/lib/query2/query/ErrorPrintingTargetEdgeErrorObserver.java:40 ErrorPrintingTargetEdgeErrorObserver.<init>
    pub fn new(event_handler: Arc<dyn EventHandler>) -> Self {
        ErrorPrintingTargetEdgeErrorObserver {
            super_: TargetEdgeErrorObserver::new(),
            event_handler,
        }
    }

    pub fn has_errors(&self) -> bool {
        self.super_.has_errors()
    }

    pub fn get_detailed_exit_code(&self) -> Option<DetailedExitCode> {
        self.super_.get_detailed_exit_code()
    }
}

impl TargetEdgeObserver for ErrorPrintingTargetEdgeErrorObserver {
    fn edge(&self, from: &Target, attribute: Option<&Attribute>, to: &Target) {
        self.super_.edge(from, attribute, to);
    }

    // Java: com/google/devtools/build/lib/query2/query/ErrorPrintingTargetEdgeErrorObserver.java:46 ErrorPrintingTargetEdgeErrorObserver.missingEdge
    fn missing_edge(&self, target: Option<&Target>, label: &Label, e: &NoSuchThingException) {
        let detailed_exit_code = e.get_detailed_exit_code();
        let location = target
            .and_then(target_utils::get_location_maybe)
            .map(|l| blaze_events::Location::new(l.file.clone(), l.line as i32, l.column as i32));
        self.event_handler.handle(
            &Event::error_at(
                location,
                target_utils::format_missing_edge(target, label, e),
            )
            .with_property(Some(detailed_exit_code)),
        );
        self.super_.missing_edge(target, label, e);
    }

    fn node(&self, node: &Target) {
        self.super_.node(node);
    }
}
