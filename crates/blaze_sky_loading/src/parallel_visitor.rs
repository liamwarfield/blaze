//! Port of `cmdline/ParallelVisitor.java`: a visitation over a graph of keys, in batches, whose
//! results are passed to a callback.
//!
//! PORT: Java runs the visit tasks and the result tasks on a thread pool (an
//! `AbstractQueueVisitor` over a `BlockingStack` work queue), so the order in which results reach
//! the callback depends on thread scheduling and differs from run to run; only the set of
//! results is fixed. Blaze runs the tasks on the calling thread, in waves: every pending key
//! is drained from the visit queue, partitioned into visit tasks (`getVisitTasks`), and each task
//! is visited in turn; its results are processed (in batches of `processResultsBatchSize`) before
//! its keys to visit join the queue for the next wave. The scheduling knobs (`minPendingTasks`,
//! `batchCallbackSize`, the 1 ms scheduling interval) and the task status callback
//! (`VisitTaskStatusCallback.NULL_INSTANCE` in Bazel) only affect the scheduling and are not
//! ported.

/// The result of one visit task: the keys whose values are results, and the keys to visit
/// next (Java `ParallelVisitor.Visit`).
// Java: com/google/devtools/build/lib/cmdline/ParallelVisitor.java:173 ParallelVisitor.Visit
pub struct Visit<OutputKeyT, VisitKeyT> {
    pub keys_to_use_for_result: Vec<OutputKeyT>,
    pub keys_to_visit: Vec<VisitKeyT>,
}

/// A visitation (Java `ParallelVisitor<InputT, VisitKeyT, OutputKeyT, OutputResultT, ExceptionT,
/// CallbackT>`); [`visit_and_wait_for_completion`] runs it.
// Java: com/google/devtools/build/lib/cmdline/ParallelVisitor.java:48 ParallelVisitor
pub trait ParallelVisitor {
    type Input;
    type VisitKey;
    type OutputKey;
    type OutputResult;
    type Error;

    /// The number of keys a visit task gets (Java `visitBatchSize`).
    fn visit_batch_size(&self) -> usize;

    /// The number of result keys a result task gets (Java `processResultsBatchSize`).
    fn process_results_batch_size(&self) -> usize;

    /// Gets the output values corresponding to the given keys.
    // Java: com/google/devtools/build/lib/cmdline/ParallelVisitor.java:169 ParallelVisitor.outputKeysToOutputValues
    fn output_keys_to_output_values(
        &self,
        target_keys: Vec<Self::OutputKey>,
    ) -> Result<Vec<Self::OutputResult>, Self::Error>;

    /// Gets the [`Visit`] representing the local visitation of the given keys.
    // Java: com/google/devtools/build/lib/cmdline/ParallelVisitor.java:190 ParallelVisitor.getVisitResult
    fn get_visit_result(
        &self,
        values: Vec<Self::VisitKey>,
    ) -> Result<Visit<Self::OutputKey, Self::VisitKey>, Self::Error>;

    /// Returns the keys to start the visitation with.
    // Java: com/google/devtools/build/lib/cmdline/ParallelVisitor.java:194 ParallelVisitor.preprocessInitialVisit
    fn preprocess_initial_visit(&self, inputs: Vec<Self::Input>) -> Vec<Self::VisitKey>;

    /// Returns the keys of `prospective_visitation_keys` not visited yet, noting them as visited.
    // Java: com/google/devtools/build/lib/cmdline/ParallelVisitor.java:201 ParallelVisitor.noteAndReturnUniqueVisitationKeys
    fn note_and_return_unique_visitation_keys(
        &self,
        prospective_visitation_keys: Vec<Self::VisitKey>,
    ) -> Result<Vec<Self::VisitKey>, Self::Error>;

    /// Splits the pending keys into visit tasks, by default batches of
    /// [`ParallelVisitor::visit_batch_size`].
    // Java: com/google/devtools/build/lib/cmdline/ParallelVisitor.java:205 ParallelVisitor.getVisitTasks
    fn get_visit_tasks(
        &self,
        pending_keys_to_visit: Vec<Self::VisitKey>,
    ) -> Result<Vec<Vec<Self::VisitKey>>, Self::Error> {
        Ok(partition(pending_keys_to_visit, self.visit_batch_size()))
    }

    /// The callback (Java `callback.process`).
    fn process(&self, partial_result: Vec<Self::OutputResult>) -> Result<(), Self::Error>;
}

/// Guava `Iterables.partition`: consecutive chunks of `size` (the last may be smaller).
pub fn partition<T>(items: Vec<T>, size: usize) -> Vec<Vec<T>> {
    let size = size.max(1);
    let mut out: Vec<Vec<T>> = Vec::new();
    let mut current: Vec<T> = Vec::new();
    for item in items {
        current.push(item);
        if current.len() == size {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Visits `keys` and everything reachable from them, passing the results to the callback.
// Java: com/google/devtools/build/lib/cmdline/ParallelVisitor.java:183 ParallelVisitor.visitAndWaitForCompletion
// PORT: skipped, the tasks run on the calling thread (see the module comment): ParallelVisitor.process, ParallelVisitor.awaitTerminationAndPropagateErrorsIfAny
// PORT: skipped, Bazel's status callback is NULL_INSTANCE (see the module comment): ParallelVisitor.onVisitTaskDiscovered, ParallelVisitor.onVisitTaskCompleted
// Java: com/google/devtools/build/lib/cmdline/ParallelVisitor.java:312 ParallelVisitor.VisitingTaskExecutor.visitAndWaitForCompletion
// Java: com/google/devtools/build/lib/cmdline/ParallelVisitor.java:352 ParallelVisitor.VisitingTaskExecutor.moreWorkToDo
// Java: com/google/devtools/build/lib/cmdline/ParallelVisitor.java:216 ParallelVisitor.addToVisitQueue
pub fn visit_and_wait_for_completion<V: ParallelVisitor + ?Sized>(
    visitor: &V,
    keys: Vec<V::Input>,
) -> Result<(), V::Error> {
    let mut visit_queue: Vec<V::VisitKey> =
        visitor.note_and_return_unique_visitation_keys(visitor.preprocess_initial_visit(keys))?;
    // Java VisitingTaskExecutor.visitAndWaitForCompletion: while there is work, drain the queue
    // into visit tasks.
    while !visit_queue.is_empty() {
        let pending_keys_to_visit = std::mem::take(&mut visit_queue);
        for keys_to_visit in visitor.get_visit_tasks(pending_keys_to_visit)? {
            // Java VisitTask.process.
            // Java: com/google/devtools/build/lib/cmdline/ParallelVisitor.java:264 ParallelVisitor.VisitTask.process
            // Java: com/google/devtools/build/lib/cmdline/ParallelVisitor.java:229 ParallelVisitor.Task.run
            let visit = visitor.get_visit_result(keys_to_visit)?;
            for keys_to_use_for_result_batch in partition(
                visit.keys_to_use_for_result,
                visitor.process_results_batch_size(),
            ) {
                // Java GetAndProcessUniqueResultsTask.process.
                // Java: com/google/devtools/build/lib/cmdline/ParallelVisitor.java:288 ParallelVisitor.GetAndProcessUniqueResultsTask.process
                visitor
                    .process(visitor.output_keys_to_output_values(keys_to_use_for_result_batch)?)?;
            }
            visit_queue
                .extend(visitor.note_and_return_unique_visitation_keys(visit.keys_to_visit)?);
        }
    }
    Ok(())
}
