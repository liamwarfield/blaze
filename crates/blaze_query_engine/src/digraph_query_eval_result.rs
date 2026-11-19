//! Port of `query2/engine/DigraphQueryEvalResult.java`.

use std::cell::{Ref, RefCell};
use std::rc::Rc;

use blaze_util::graph::Digraph;

use crate::query_eval_result::QueryEvalResult;

/// `QueryEvalResult` along with a digraph giving the structure of the results.
///
/// PORT: the graph is shared with the environment that built it (Java hands out the
/// environment's own `Digraph`).
// Java: com/google/devtools/build/lib/query2/engine/DigraphQueryEvalResult.java:22 DigraphQueryEvalResult
pub struct DigraphQueryEvalResult<T> {
    result: QueryEvalResult,
    graph: Rc<RefCell<Digraph<T>>>,
}

impl<T> DigraphQueryEvalResult<T> {
    // Java: com/google/devtools/build/lib/query2/engine/DigraphQueryEvalResult.java:26 DigraphQueryEvalResult.<init>
    pub fn new(result: QueryEvalResult, graph: Rc<RefCell<Digraph<T>>>) -> Self {
        DigraphQueryEvalResult { result, graph }
    }

    /// The `QueryEvalResult` part (Java: the superclass).
    pub fn get_result(&self) -> &QueryEvalResult {
        &self.result
    }

    /// Returns the recorded graph
    // Java: com/google/devtools/build/lib/query2/engine/DigraphQueryEvalResult.java:33 DigraphQueryEvalResult.getGraph
    pub fn get_graph(&self) -> Ref<'_, Digraph<T>> {
        self.graph.borrow()
    }
}
