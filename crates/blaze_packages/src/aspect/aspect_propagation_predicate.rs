//! Port of `packages/AspectPropagationPredicate.java`.

use std::fmt;
use std::sync::Arc;

use blaze_events::EventHandler;
use blaze_starlark::EvalError;
use blaze_starlark::StarlarkSemantics;

use super::aspect_propagation_edges_supplier::StarlarkAspectPropagationContext;
use super::aspect_propagation_edges_supplier::eval_exception_message;
use super::aspect_propagation_edges_supplier::run_transient_call;
use crate::rule_class::StarlarkCallable;
use crate::starlark_defs::value_cell::same_object;

/// Starlark function that determines whether an aspect should be propagated to the target (Java
/// `AspectPropagationPredicate`).
// Java: com/google/devtools/build/lib/packages/AspectPropagationPredicate.java:30 AspectPropagationPredicate
#[derive(Clone)]
pub struct AspectPropagationPredicate {
    predicate: StarlarkCallable,
    semantics: Arc<StarlarkSemantics>,
}

impl AspectPropagationPredicate {
    // Java: com/google/devtools/build/lib/packages/AspectPropagationPredicate.java:35 AspectPropagationPredicate.<init>
    pub fn new(
        predicate: StarlarkCallable,
        semantics: Arc<StarlarkSemantics>,
    ) -> AspectPropagationPredicate {
        AspectPropagationPredicate {
            predicate,
            semantics,
        }
    }

    /// The Starlark function.
    pub fn get_predicate(&self) -> StarlarkCallable {
        self.predicate
    }

    pub fn get_semantics(&self) -> &Arc<StarlarkSemantics> {
        &self.semantics
    }

    // Java: com/google/devtools/build/lib/packages/AspectPropagationPredicate.java:40 AspectPropagationPredicate.evaluate
    pub fn evaluate(
        &self,
        context: StarlarkAspectPropagationContext<'_>,
        event_handler: &dyn EventHandler,
    ) -> Result<bool, EvalError> {
        // Java: runPropagationPredicate (AspectPropagationPredicate.java:52), then the
        // `instanceof Boolean` check on its result.
        run_transient_call(
            self.predicate,
            &self.semantics,
            context,
            event_handler,
            |starlark_result| match starlark_result.unpack_bool() {
                Some(boolean_result) => Ok(boolean_result),
                None => Err(eval_exception_message("Expected a boolean")),
            },
        )
    }
}

impl PartialEq for AspectPropagationPredicate {
    // Java: com/google/devtools/build/lib/packages/AspectPropagationPredicate.java:71 AspectPropagationPredicate.equals
    // PORT: `StarlarkFunction` equality is identity.
    fn eq(&self, other: &AspectPropagationPredicate) -> bool {
        same_object(self.predicate, other.predicate) && self.semantics == other.semantics
    }
}

impl Eq for AspectPropagationPredicate {}

impl fmt::Debug for AspectPropagationPredicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("AspectPropagationPredicate")
            .field(&self.predicate)
            .finish()
    }
}
