//! The Java tests of this package that are not ported, one ignored stub each, under the Java
//! name. Running one (`--ignored`) fails with the reason: none of them passes silently.
//!
//! Generated from the `@Test` methods of `ParallelEvaluatorTest` and `MemoizingEvaluatorTest`
//! at the pin that have no `// Java: <Class>.<method>` port in this directory or in `tests.rs`.

mod parallel_evaluator_test {
    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:238 ParallelEvaluatorTest.enqueueDoneFuture
    #[test]
    #[ignore = "Environment.dependOnFuture (external deps) is not ported"]
    fn enqueue_done_future() {
        unimplemented!("not ported: ParallelEvaluatorTest.enqueueDoneFuture")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:262 ParallelEvaluatorTest.enqueueBadFuture
    #[test]
    #[ignore = "Environment.dependOnFuture (external deps) is not ported"]
    fn enqueue_bad_future() {
        unimplemented!("not ported: ParallelEvaluatorTest.enqueueBadFuture")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:310 ParallelEvaluatorTest.dependsOnKeyAndFuture
    #[test]
    #[ignore = "Environment.dependOnFuture (external deps) is not ported"]
    fn depends_on_key_and_future() {
        unimplemented!("not ported: ParallelEvaluatorTest.dependsOnKeyAndFuture")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:419 ParallelEvaluatorTest.interruptedEvaluatorThreadAfterEnqueueBeforeWaitForCompletionAndConstructResult
    #[test]
    #[ignore = "needs a graph listener (NotifyingHelper), which the graph here does not have"]
    fn interrupted_evaluator_thread_after_enqueue_before_wait_for_completion_and_construct_result()
    {
        unimplemented!(
            "not ported: ParallelEvaluatorTest.interruptedEvaluatorThreadAfterEnqueueBeforeWaitForCompletionAndConstructResult"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:501 ParallelEvaluatorTest.runPartialResultOnInterruption
    #[test]
    #[ignore = "EvaluationProgressReceiver is not ported"]
    fn run_partial_result_on_interruption() {
        unimplemented!("not ported: ParallelEvaluatorTest.runPartialResultOnInterruption")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:760 ParallelEvaluatorTest.fullEventStorage_unstoredEvent_reportedImmediately_notReplayed
    #[test]
    #[ignore = "events stored on nodes and replayed (EventFilter) are not ported: one build per \
                process"]
    fn full_event_storage_unstored_event_reported_immediately_not_replayed() {
        unimplemented!(
            "not ported: ParallelEvaluatorTest.fullEventStorage_unstoredEvent_reportedImmediately_notReplayed"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:795 ParallelEvaluatorTest.fullEventStorage_storedEvent_reportedAfterSkyFunctionCompletes_replayed
    #[test]
    #[ignore = "events stored on nodes and replayed (EventFilter) are not ported: one build per \
                process"]
    fn full_event_storage_stored_event_reported_after_sky_function_completes_replayed() {
        unimplemented!(
            "not ported: ParallelEvaluatorTest.fullEventStorage_storedEvent_reportedAfterSkyFunctionCompletes_replayed"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:2406 ParallelEvaluatorTest.declareDependenciesAndCheckIfValuesMissing
    #[test]
    #[ignore = "GraphTraversingHelper is not ported"]
    fn declare_dependencies_and_check_if_values_missing() {
        unimplemented!(
            "not ported: ParallelEvaluatorTest.declareDependenciesAndCheckIfValuesMissing"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:2447 ParallelEvaluatorTest.declareDependenciesAndCheckIfNotValuesMissing
    #[test]
    #[ignore = "GraphTraversingHelper is not ported"]
    fn declare_dependencies_and_check_if_not_values_missing() {
        unimplemented!(
            "not ported: ParallelEvaluatorTest.declareDependenciesAndCheckIfNotValuesMissing"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:2488 ParallelEvaluatorTest.validateExceptionTypeInDifferentPosition
    #[test]
    #[ignore = "SkyFunctionException.validateExceptionType checks Java exception classes; \
                get_value_or_throw takes one concrete Rust error type"]
    fn validate_exception_type_in_different_position() {
        unimplemented!("not ported: ParallelEvaluatorTest.validateExceptionTypeInDifferentPosition")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:2527 ParallelEvaluatorTest.validateExceptionTypeWithDifferentException
    #[test]
    #[ignore = "SkyFunctionException.validateExceptionType checks Java exception classes; \
                get_value_or_throw takes one concrete Rust error type"]
    fn validate_exception_type_with_different_exception() {
        unimplemented!(
            "not ported: ParallelEvaluatorTest.validateExceptionTypeWithDifferentException"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:2600 ParallelEvaluatorTest.signalValueEnqueuedAndEvaluated
    #[test]
    #[ignore = "EvaluationProgressReceiver is not ported"]
    fn signal_value_enqueued_and_evaluated() {
        unimplemented!("not ported: ParallelEvaluatorTest.signalValueEnqueuedAndEvaluated")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:2880 ParallelEvaluatorTest.declaresDifferentDepsAfterRestart
    #[test]
    #[ignore = "needs the restart model: functions are not restarted in normal evaluation, a \
                request computes the dep"]
    fn declares_different_deps_after_restart() {
        unimplemented!("not ported: ParallelEvaluatorTest.declaresDifferentDepsAfterRestart")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:3014 ParallelEvaluatorTest.skyKeyComputeState
    #[test]
    #[ignore = "SkyKeyComputeState (Environment.getState) is not ported"]
    fn sky_key_compute_state() {
        unimplemented!("not ported: ParallelEvaluatorTest.skyKeyComputeState")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:3110 ParallelEvaluatorTest.skyKeyComputeState_noKeepGoingWithAnError
    #[test]
    #[ignore = "SkyKeyComputeState (Environment.getState) is not ported"]
    fn sky_key_compute_state_no_keep_going_with_an_error() {
        unimplemented!(
            "not ported: ParallelEvaluatorTest.skyKeyComputeState_noKeepGoingWithAnError"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:3186 ParallelEvaluatorTest.skyKeyComputeState_unnecessaryTemporaryStateDropperReceiver
    #[test]
    #[ignore = "SkyKeyComputeState (Environment.getState) is not ported"]
    fn sky_key_compute_state_unnecessary_temporary_state_dropper_receiver() {
        unimplemented!(
            "not ported: ParallelEvaluatorTest.skyKeyComputeState_unnecessaryTemporaryStateDropperReceiver"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:3302 ParallelEvaluatorTest.classToInstanceMapSkyKeyComputeState
    #[test]
    #[ignore = "SkyKeyComputeState (Environment.getState) is not ported"]
    fn class_to_instance_map_sky_key_compute_state() {
        unimplemented!("not ported: ParallelEvaluatorTest.classToInstanceMapSkyKeyComputeState")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:3442 ParallelEvaluatorTest.partialReevaluationOneButNotAllDeps
    #[test]
    #[ignore = "partial reevaluation (SkyKey.supportsPartialReevaluation) is not ported"]
    fn partial_reevaluation_one_but_not_all_deps() {
        unimplemented!("not ported: ParallelEvaluatorTest.partialReevaluationOneButNotAllDeps")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:3517 ParallelEvaluatorTest.partialReevaluationOneDuringAReevaluation
    #[test]
    #[ignore = "partial reevaluation (SkyKey.supportsPartialReevaluation) is not ported"]
    fn partial_reevaluation_one_during_a_reevaluation() {
        unimplemented!(
            "not ported: ParallelEvaluatorTest.partialReevaluationOneDuringAReevaluation"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:3676 ParallelEvaluatorTest.partialReevaluationErrorDuringReevaluation
    #[test]
    #[ignore = "partial reevaluation (SkyKey.supportsPartialReevaluation) is not ported"]
    fn partial_reevaluation_error_during_reevaluation() {
        unimplemented!(
            "not ported: ParallelEvaluatorTest.partialReevaluationErrorDuringReevaluation"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:3829 ParallelEvaluatorTest.partialReevaluationOneErrorButNotAllDeps
    #[test]
    #[ignore = "partial reevaluation (SkyKey.supportsPartialReevaluation) is not ported"]
    fn partial_reevaluation_one_error_but_not_all_deps() {
        unimplemented!("not ported: ParallelEvaluatorTest.partialReevaluationOneErrorButNotAllDeps")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:3949 ParallelEvaluatorTest.partialReevaluationErrorObservedDuringReevaluation
    #[test]
    #[ignore = "partial reevaluation (SkyKey.supportsPartialReevaluation) is not ported"]
    fn partial_reevaluation_error_observed_during_reevaluation() {
        unimplemented!(
            "not ported: ParallelEvaluatorTest.partialReevaluationErrorObservedDuringReevaluation"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:4151 ParallelEvaluatorTest.testParallelSkyFunctionComputation_runnablesOnBothCurrentAndExternalThreads
    #[test]
    #[ignore = "needs an injected QuiescingExecutor (ParallelEvaluator's executor parameter), \
                not ported"]
    fn test_parallel_sky_function_computation_runnables_on_both_current_and_external_threads() {
        unimplemented!(
            "not ported: ParallelEvaluatorTest.testParallelSkyFunctionComputation_runnablesOnBothCurrentAndExternalThreads"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:4206 ParallelEvaluatorTest.testParallelSkyFunctionComputation_runnablesOnExternalThreadsOnly
    #[test]
    #[ignore = "needs an injected QuiescingExecutor (ParallelEvaluator's executor parameter), \
                not ported"]
    fn test_parallel_sky_function_computation_runnables_on_external_threads_only() {
        unimplemented!(
            "not ported: ParallelEvaluatorTest.testParallelSkyFunctionComputation_runnablesOnExternalThreadsOnly"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:4239 ParallelEvaluatorTest.customKeepGoingPredicate_sameForAllKeys
    #[test]
    #[ignore = "the per-key keep_going predicate is not ported"]
    fn custom_keep_going_predicate_same_for_all_keys() {
        unimplemented!("not ported: ParallelEvaluatorTest.customKeepGoingPredicate_sameForAllKeys")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:4283 ParallelEvaluatorTest.customKeepGoingPredicate_differentPerKey
    #[test]
    #[ignore = "the per-key keep_going predicate is not ported"]
    fn custom_keep_going_predicate_different_per_key() {
        unimplemented!("not ported: ParallelEvaluatorTest.customKeepGoingPredicate_differentPerKey")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:4322 ParallelEvaluatorTest.topLevelKeyBuiltAsDepOfAnotherTopLevelKey_reportedToProgressReceiverOnce
    #[test]
    #[ignore = "EvaluationProgressReceiver is not ported"]
    fn top_level_key_built_as_dep_of_another_top_level_key_reported_to_progress_receiver_once() {
        unimplemented!(
            "not ported: ParallelEvaluatorTest.topLevelKeyBuiltAsDepOfAnotherTopLevelKey_reportedToProgressReceiverOnce"
        )
    }
}

mod memoizing_evaluator_test {
    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:186 MemoizingEvaluatorTest.injectGraphTransformer_transformedGraphUsedForInMemoryGraph
    #[test]
    #[ignore = "graph transformers for testing (injectGraphTransformerForTesting) are not ported"]
    fn inject_graph_transformer_transformed_graph_used_for_in_memory_graph() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.injectGraphTransformer_transformedGraphUsedForInMemoryGraph"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:209 MemoizingEvaluatorTest.injectGraphTransformer_transformedGraphUsedForEvaluation
    #[test]
    #[ignore = "graph transformers for testing (injectGraphTransformerForTesting) are not ported"]
    fn inject_graph_transformer_transformed_graph_used_for_evaluation() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.injectGraphTransformer_transformedGraphUsedForEvaluation"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:223 MemoizingEvaluatorTest.injectGraphTransformer_multipleTransformersAppliedInOrder
    #[test]
    #[ignore = "graph transformers for testing (injectGraphTransformerForTesting) are not ported"]
    fn inject_graph_transformer_multiple_transformers_applied_in_order() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.injectGraphTransformer_multipleTransformersAppliedInOrder"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:244 MemoizingEvaluatorTest.invalidationWithNothingChanged
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn invalidation_with_nothing_changed() {
        unimplemented!("not ported: MemoizingEvaluatorTest.invalidationWithNothingChanged")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:443 MemoizingEvaluatorTest.deleteValues
    #[test]
    #[ignore = "MemoizingEvaluator.delete / deleteDirty are not ported"]
    fn delete_values() {
        unimplemented!("not ported: MemoizingEvaluatorTest.deleteValues")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:469 MemoizingEvaluatorTest.deleteOldNodesTest
    #[test]
    #[ignore = "MemoizingEvaluator.delete / deleteDirty are not ported"]
    fn delete_old_nodes_test() {
        unimplemented!("not ported: MemoizingEvaluatorTest.deleteOldNodesTest")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:505 MemoizingEvaluatorTest.deleteDirtyCleanedValue
    #[test]
    #[ignore = "MemoizingEvaluator.delete / deleteDirty are not ported"]
    fn delete_dirty_cleaned_value() {
        unimplemented!("not ported: MemoizingEvaluatorTest.deleteDirtyCleanedValue")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:517 MemoizingEvaluatorTest.deleteNonexistentValues
    #[test]
    #[ignore = "MemoizingEvaluator.delete / deleteDirty are not ported"]
    fn delete_nonexistent_values() {
        unimplemented!("not ported: MemoizingEvaluatorTest.deleteNonexistentValues")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:525 MemoizingEvaluatorTest.signalValueEnqueued
    #[test]
    #[ignore = "EvaluationProgressReceiver is not ported"]
    fn signal_value_enqueued() {
        unimplemented!("not ported: MemoizingEvaluatorTest.signalValueEnqueued")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:717 MemoizingEvaluatorTest.invalidationWithChangeAndThenNothingChanged
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn invalidation_with_change_and_then_nothing_changed() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.invalidationWithChangeAndThenNothingChanged"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:733 MemoizingEvaluatorTest.noKeepGoingErrorAfterKeepGoingError
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn no_keep_going_error_after_keep_going_error() {
        unimplemented!("not ported: MemoizingEvaluatorTest.noKeepGoingErrorAfterKeepGoingError")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:778 MemoizingEvaluatorTest.transientPruning
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn transient_pruning() {
        unimplemented!("not ported: MemoizingEvaluatorTest.transientPruning")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:843 MemoizingEvaluatorTest.alreadyAnalyzedBadTarget
    #[test]
    #[ignore = "EvaluationProgressReceiver is not ported"]
    fn already_analyzed_bad_target() {
        unimplemented!("not ported: MemoizingEvaluatorTest.alreadyAnalyzedBadTarget")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:875 MemoizingEvaluatorTest.receiverToldOfVerifiedValueDependingOnCycle
    #[test]
    #[ignore = "EvaluationProgressReceiver is not ported"]
    fn receiver_told_of_verified_value_depending_on_cycle() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.receiverToldOfVerifiedValueDependingOnCycle"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:892 MemoizingEvaluatorTest.incrementalAddedDependency
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn incremental_added_dependency() {
        unimplemented!("not ported: MemoizingEvaluatorTest.incrementalAddedDependency")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1096 MemoizingEvaluatorTest.changeCycle_NoKeepGoing
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn change_cycle_no_keep_going() {
        unimplemented!("not ported: MemoizingEvaluatorTest.changeCycle_NoKeepGoing")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1101 MemoizingEvaluatorTest.changeCycle_KeepGoing
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn change_cycle_keep_going() {
        unimplemented!("not ported: MemoizingEvaluatorTest.changeCycle_KeepGoing")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1180 MemoizingEvaluatorTest.cycleAndSelfEdgeWithDirtyValue
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn cycle_and_self_edge_with_dirty_value() {
        unimplemented!("not ported: MemoizingEvaluatorTest.cycleAndSelfEdgeWithDirtyValue")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1255 MemoizingEvaluatorTest.cycleWithDirtyValue
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn cycle_with_dirty_value() {
        unimplemented!("not ported: MemoizingEvaluatorTest.cycleWithDirtyValue")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1393 MemoizingEvaluatorTest.cycleAndErrorAndReady
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn cycle_and_error_and_ready() {
        unimplemented!("not ported: MemoizingEvaluatorTest.cycleAndErrorAndReady")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1502 MemoizingEvaluatorTest.breakCycle
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn break_cycle() {
        unimplemented!("not ported: MemoizingEvaluatorTest.breakCycle")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1546 MemoizingEvaluatorTest.nodeInvalidatedThenDoubleCycle
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn node_invalidated_then_double_cycle() {
        unimplemented!("not ported: MemoizingEvaluatorTest.nodeInvalidatedThenDoubleCycle")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1660 MemoizingEvaluatorTest.nodeIsChangedWithoutBeingEvaluated
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn node_is_changed_without_being_evaluated() {
        unimplemented!("not ported: MemoizingEvaluatorTest.nodeIsChangedWithoutBeingEvaluated")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1708 MemoizingEvaluatorTest.incompleteDirectDepsAreClearedBeforeInvalidation
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn incomplete_direct_deps_are_cleared_before_invalidation() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.incompleteDirectDepsAreClearedBeforeInvalidation"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1866 MemoizingEvaluatorTest.incompleteDirectDepsForDirtyValue
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn incomplete_direct_deps_for_dirty_value() {
        unimplemented!("not ported: MemoizingEvaluatorTest.incompleteDirectDepsForDirtyValue")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1947 MemoizingEvaluatorTest.continueWithErrorDepTurnedGood
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn continue_with_error_dep_turned_good() {
        unimplemented!("not ported: MemoizingEvaluatorTest.continueWithErrorDepTurnedGood")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1968 MemoizingEvaluatorTest.errorDepAlreadyThereThenTurnedGood
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn error_dep_already_there_then_turned_good() {
        unimplemented!("not ported: MemoizingEvaluatorTest.errorDepAlreadyThereThenTurnedGood")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2340 MemoizingEvaluatorTest.dirtyChildEnqueuesParentDuringCheckDependencies_ThrowDoesntEnqueue
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn dirty_child_enqueues_parent_during_check_dependencies_throw_doesnt_enqueue() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.dirtyChildEnqueuesParentDuringCheckDependencies_ThrowDoesntEnqueue"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2346 MemoizingEvaluatorTest.dirtyChildEnqueuesParentDuringCheckDependencies_NoThrow
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn dirty_child_enqueues_parent_during_check_dependencies_no_throw() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.dirtyChildEnqueuesParentDuringCheckDependencies_NoThrow"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2351 MemoizingEvaluatorTest.removeReverseDepFromRebuildingNode
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn remove_reverse_dep_from_rebuilding_node() {
        unimplemented!("not ported: MemoizingEvaluatorTest.removeReverseDepFromRebuildingNode")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2414 MemoizingEvaluatorTest.dirtyThenDeleted
    #[test]
    #[ignore = "MemoizingEvaluator.delete / deleteDirty are not ported"]
    fn dirty_then_deleted() {
        unimplemented!("not ported: MemoizingEvaluatorTest.dirtyThenDeleted")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2444 MemoizingEvaluatorTest.resetSelfOnly_singleDep_initialBuild
    #[test]
    #[ignore = "SkyFunction.Reset and node rewinding are not ported"]
    fn reset_self_only_single_dep_initial_build() {
        unimplemented!("not ported: MemoizingEvaluatorTest.resetSelfOnly_singleDep_initialBuild")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2491 MemoizingEvaluatorTest.resetSelfOnly_singleDep_incrementalBuild
    #[test]
    #[ignore = "SkyFunction.Reset and node rewinding are not ported"]
    fn reset_self_only_single_dep_incremental_build() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.resetSelfOnly_singleDep_incrementalBuild"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2547 MemoizingEvaluatorTest.resetSelfOnly_extraDepMissingAfterReset_initialBuild
    #[test]
    #[ignore = "SkyFunction.Reset and node rewinding are not ported"]
    fn reset_self_only_extra_dep_missing_after_reset_initial_build() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.resetSelfOnly_extraDepMissingAfterReset_initialBuild"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2604 MemoizingEvaluatorTest.resetSelfOnly_extraDepMissingAfterReset_incrementalBuild
    #[test]
    #[ignore = "SkyFunction.Reset and node rewinding are not ported"]
    fn reset_self_only_extra_dep_missing_after_reset_incremental_build() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.resetSelfOnly_extraDepMissingAfterReset_incrementalBuild"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2664 MemoizingEvaluatorTest.resetSelfOnly_depNotRequestedAgainAfterReset
    #[test]
    #[ignore = "SkyFunction.Reset and node rewinding are not ported"]
    fn reset_self_only_dep_not_requested_again_after_reset() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.resetSelfOnly_depNotRequestedAgainAfterReset"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2724 MemoizingEvaluatorTest.resetSelfOnly_evaluationAborted
    #[test]
    #[ignore = "SkyFunction.Reset and node rewinding are not ported"]
    fn reset_self_only_evaluation_aborted() {
        unimplemented!("not ported: MemoizingEvaluatorTest.resetSelfOnly_evaluationAborted")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2768 MemoizingEvaluatorTest.rewindOneDep
    #[test]
    #[ignore = "SkyFunction.Reset and node rewinding are not ported"]
    fn rewind_one_dep() {
        unimplemented!("not ported: MemoizingEvaluatorTest.rewindOneDep")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2816 MemoizingEvaluatorTest.twoParentsRewindSameDep_markedDirtyOnce
    #[test]
    #[ignore = "SkyFunction.Reset and node rewinding are not ported"]
    fn two_parents_rewind_same_dep_marked_dirty_once() {
        unimplemented!("not ported: MemoizingEvaluatorTest.twoParentsRewindSameDep_markedDirtyOnce")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2879 MemoizingEvaluatorTest.twoParentsRewindSameDep_markedDirtyTwice
    #[test]
    #[ignore = "SkyFunction.Reset and node rewinding are not ported"]
    fn two_parents_rewind_same_dep_marked_dirty_twice() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.twoParentsRewindSameDep_markedDirtyTwice"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2956 MemoizingEvaluatorTest.twoParentsRewindSameDep_depEvaluatesToErrorAfterRewind
    #[test]
    #[ignore = "SkyFunction.Reset and node rewinding are not ported"]
    fn two_parents_rewind_same_dep_dep_evaluates_to_error_after_rewind() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.twoParentsRewindSameDep_depEvaluatesToErrorAfterRewind"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3041 MemoizingEvaluatorTest.depWithDoneParentEvaluatesToErrorAfterRewind_reverseTransitiveClosureDeleted
    #[test]
    #[ignore = "SkyFunction.Reset and node rewinding are not ported"]
    fn dep_with_done_parent_evaluates_to_error_after_rewind_reverse_transitive_closure_deleted() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.depWithDoneParentEvaluatesToErrorAfterRewind_reverseTransitiveClosureDeleted"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3122 MemoizingEvaluatorTest.rewindTransitiveDep
    #[test]
    #[ignore = "SkyFunction.Reset and node rewinding are not ported"]
    fn rewind_transitive_dep() {
        unimplemented!("not ported: MemoizingEvaluatorTest.rewindTransitiveDep")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3194 MemoizingEvaluatorTest.evaluationAbortedWithRewoundNodeOnInvalidationPath_dirty
    #[test]
    #[ignore = "SkyFunction.Reset and node rewinding are not ported"]
    fn evaluation_aborted_with_rewound_node_on_invalidation_path_dirty() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.evaluationAbortedWithRewoundNodeOnInvalidationPath_dirty"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3262 MemoizingEvaluatorTest.evaluationAbortedWithRewoundNodeOnInvalidationPath_inflight
    #[test]
    #[ignore = "SkyFunction.Reset and node rewinding are not ported"]
    fn evaluation_aborted_with_rewound_node_on_invalidation_path_inflight() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.evaluationAbortedWithRewoundNodeOnInvalidationPath_inflight"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3447 MemoizingEvaluatorTest.dirtyAndChanged
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn dirty_and_changed() {
        unimplemented!("not ported: MemoizingEvaluatorTest.dirtyAndChanged")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3478 MemoizingEvaluatorTest.dirtyAndChangedValueIsChanged
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn dirty_and_changed_value_is_changed() {
        unimplemented!("not ported: MemoizingEvaluatorTest.dirtyAndChangedValueIsChanged")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3568 MemoizingEvaluatorTest.singleValueDependsOnManyDirtyValues
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn single_value_depends_on_many_dirty_values() {
        unimplemented!("not ported: MemoizingEvaluatorTest.singleValueDependsOnManyDirtyValues")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3708 MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithInvalidateRemoveError
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn dirty_children_properly_removed_with_invalidate_remove_error() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithInvalidateRemoveError"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3714 MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithInvalidateKeepError
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn dirty_children_properly_removed_with_invalidate_keep_error() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithInvalidateKeepError"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3720 MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithReevaluateRemoveError
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn dirty_children_properly_removed_with_reevaluate_remove_error() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithReevaluateRemoveError"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3726 MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithReevaluateKeepError
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn dirty_children_properly_removed_with_reevaluate_keep_error() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithReevaluateKeepError"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3816 MemoizingEvaluatorTest.manyDirtyValuesClearChildrenOnError
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn many_dirty_values_clear_children_on_error() {
        unimplemented!("not ported: MemoizingEvaluatorTest.manyDirtyValuesClearChildrenOnError")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3825 MemoizingEvaluatorTest.manyDirtyValuesClearChildrenOnInterrupt
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn many_dirty_values_clear_children_on_interrupt() {
        unimplemented!("not ported: MemoizingEvaluatorTest.manyDirtyValuesClearChildrenOnInterrupt")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3851 MemoizingEvaluatorTest.deletingDirtyNodes
    #[test]
    #[ignore = "MemoizingEvaluator.delete / deleteDirty are not ported"]
    fn deleting_dirty_nodes() {
        unimplemented!("not ported: MemoizingEvaluatorTest.deletingDirtyNodes")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3877 MemoizingEvaluatorTest.changePruning
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn change_pruning() {
        unimplemented!("not ported: MemoizingEvaluatorTest.changePruning")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3901 MemoizingEvaluatorTest.changePruningWithDoneValue
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn change_pruning_with_done_value() {
        unimplemented!("not ported: MemoizingEvaluatorTest.changePruningWithDoneValue")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3933 MemoizingEvaluatorTest.changePruningAfterParentPrunes
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn change_pruning_after_parent_prunes() {
        unimplemented!("not ported: MemoizingEvaluatorTest.changePruningAfterParentPrunes")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3976 MemoizingEvaluatorTest.changePruningFromOtherNodeAfterParentPrunes
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn change_pruning_from_other_node_after_parent_prunes() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.changePruningFromOtherNodeAfterParentPrunes"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4024 MemoizingEvaluatorTest.changedChildChangesDepOfParent
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn changed_child_changes_dep_of_parent() {
        unimplemented!("not ported: MemoizingEvaluatorTest.changedChildChangesDepOfParent")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4066 MemoizingEvaluatorTest.dirtyDepIgnoresChildren
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn dirty_dep_ignores_children() {
        unimplemented!("not ported: MemoizingEvaluatorTest.dirtyDepIgnoresChildren")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4116 MemoizingEvaluatorTest.dirtyBuildAfterFailedBuild
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn dirty_build_after_failed_build() {
        unimplemented!("not ported: MemoizingEvaluatorTest.dirtyBuildAfterFailedBuild")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4140 MemoizingEvaluatorTest.changedBuildAfterFailedThenSuccessfulBuild
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn changed_build_after_failed_then_successful_build() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.changedBuildAfterFailedThenSuccessfulBuild"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4177 MemoizingEvaluatorTest.manyDirtyValuesClearChildrenOnSecondFail
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn many_dirty_values_clear_children_on_second_fail() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.manyDirtyValuesClearChildrenOnSecondFail"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4210 MemoizingEvaluatorTest.failedDirtyBuild
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn failed_dirty_build() {
        unimplemented!("not ported: MemoizingEvaluatorTest.failedDirtyBuild")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4231 MemoizingEvaluatorTest.failedDirtyBuildInBuilder
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn failed_dirty_build_in_builder() {
        unimplemented!("not ported: MemoizingEvaluatorTest.failedDirtyBuildInBuilder")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4271 MemoizingEvaluatorTest.dirtyDependsOnErrorTurningGood
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn dirty_depends_on_error_turning_good() {
        unimplemented!("not ported: MemoizingEvaluatorTest.dirtyDependsOnErrorTurningGood")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4289 MemoizingEvaluatorTest.dirtyWithOwnErrorDependsOnTransientErrorTurningGood
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn dirty_with_own_error_depends_on_transient_error_turning_good() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.dirtyWithOwnErrorDependsOnTransientErrorTurningGood"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4394 MemoizingEvaluatorTest.dirtyWithRecoveryErrorDependsOnErrorTurningGood
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn dirty_with_recovery_error_depends_on_error_turning_good() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.dirtyWithRecoveryErrorDependsOnErrorTurningGood"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4469 MemoizingEvaluatorTest.raceClearingIncompleteValues
    #[test]
    #[ignore = "needs a graph listener (NotifyingHelper), which the graph here does not have"]
    fn race_clearing_incomplete_values() {
        unimplemented!("not ported: MemoizingEvaluatorTest.raceClearingIncompleteValues")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4637 MemoizingEvaluatorTest.notComparableNotPrunedNoEvent
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn not_comparable_not_pruned_no_event() {
        unimplemented!("not ported: MemoizingEvaluatorTest.notComparableNotPrunedNoEvent")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4642 MemoizingEvaluatorTest.notComparableNotPrunedEvent
    #[test]
    #[ignore = "events stored on nodes and replayed (EventFilter) are not ported: one build per \
                process"]
    fn not_comparable_not_pruned_event() {
        unimplemented!("not ported: MemoizingEvaluatorTest.notComparableNotPrunedEvent")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4687 MemoizingEvaluatorTest.changePruningWithEvent
    #[test]
    #[ignore = "events stored on nodes and replayed (EventFilter) are not ported: one build per \
                process"]
    fn change_pruning_with_event() {
        unimplemented!("not ported: MemoizingEvaluatorTest.changePruningWithEvent")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4712 MemoizingEvaluatorTest.changePruningWithIntermittentEvent
    #[test]
    #[ignore = "events stored on nodes and replayed (EventFilter) are not ported: one build per \
                process"]
    fn change_pruning_with_intermittent_event() {
        unimplemented!("not ported: MemoizingEvaluatorTest.changePruningWithIntermittentEvent")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4775 MemoizingEvaluatorTest.depEventPredicate
    #[test]
    #[ignore = "events stored on nodes and replayed (EventFilter) are not ported: one build per \
                process"]
    fn dep_event_predicate() {
        unimplemented!("not ported: MemoizingEvaluatorTest.depEventPredicate")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4890 MemoizingEvaluatorTest.deleteInvalidatedValue
    #[test]
    #[ignore = "MemoizingEvaluator.delete / deleteDirty are not ported"]
    fn delete_invalidated_value() {
        unimplemented!("not ported: MemoizingEvaluatorTest.deleteInvalidatedValue")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4915 MemoizingEvaluatorTest.twoRailLeftRightDependenciesWithFailure
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn two_rail_left_right_dependencies_with_failure() {
        unimplemented!("not ported: MemoizingEvaluatorTest.twoRailLeftRightDependenciesWithFailure")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4986 MemoizingEvaluatorTest.valueInjectionOverExistingDirtyEntry
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn value_injection_over_existing_dirty_entry() {
        unimplemented!("not ported: MemoizingEvaluatorTest.valueInjectionOverExistingDirtyEntry")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5003 MemoizingEvaluatorTest.valueInjectionOverExistingEntryMarkedForInvalidation
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn value_injection_over_existing_entry_marked_for_invalidation() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.valueInjectionOverExistingEntryMarkedForInvalidation"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5014 MemoizingEvaluatorTest.valueInjectionOverExistingEntryMarkedForDeletion
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn value_injection_over_existing_entry_marked_for_deletion() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.valueInjectionOverExistingEntryMarkedForDeletion"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5025 MemoizingEvaluatorTest.valueInjectionOverExistingEqualEntryMarkedForInvalidation
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn value_injection_over_existing_equal_entry_marked_for_invalidation() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.valueInjectionOverExistingEqualEntryMarkedForInvalidation"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5038 MemoizingEvaluatorTest.valueInjectionOverExistingEqualEntryMarkedForDeletion
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn value_injection_over_existing_equal_entry_marked_for_deletion() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.valueInjectionOverExistingEqualEntryMarkedForDeletion"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5051 MemoizingEvaluatorTest.valueInjectionOverValueWithDeps
    #[test]
    #[ignore = "Java ignores an injected value for a node that has deps; Evaluator::inject \
                always replaces the node (see its PORT note)"]
    fn value_injection_over_value_with_deps() {
        unimplemented!("not ported: MemoizingEvaluatorTest.valueInjectionOverValueWithDeps")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5069 MemoizingEvaluatorTest.valueInjectionOverEqualValueWithDeps
    #[test]
    #[ignore = "needs value equality (pruneInjectedValues); values here have none"]
    fn value_injection_over_equal_value_with_deps() {
        unimplemented!("not ported: MemoizingEvaluatorTest.valueInjectionOverEqualValueWithDeps")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5118 MemoizingEvaluatorTest.valueInjectionOverExistingEqualEntryDoesNotInvalidate
    #[test]
    #[ignore = "needs value equality (pruneInjectedValues); values here have none"]
    fn value_injection_over_existing_equal_entry_does_not_invalidate() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.valueInjectionOverExistingEqualEntryDoesNotInvalidate"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5205 MemoizingEvaluatorTest.shutDownBuildOnCachedError_Done
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn shut_down_build_on_cached_error_done() {
        unimplemented!("not ported: MemoizingEvaluatorTest.shutDownBuildOnCachedError_Done")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5274 MemoizingEvaluatorTest.shutDownBuildOnCachedError_Verified
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn shut_down_build_on_cached_error_verified() {
        unimplemented!("not ported: MemoizingEvaluatorTest.shutDownBuildOnCachedError_Verified")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5396 MemoizingEvaluatorTest.cachedErrorCausesRestart
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn cached_error_causes_restart() {
        unimplemented!("not ported: MemoizingEvaluatorTest.cachedErrorCausesRestart")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5559 MemoizingEvaluatorTest.removedNodeComesBackAndInvalidates
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn removed_node_comes_back_and_invalidates() {
        unimplemented!("not ported: MemoizingEvaluatorTest.removedNodeComesBackAndInvalidates")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5572 MemoizingEvaluatorTest.removedNodeComesBackAndOtherInvalidates
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn removed_node_comes_back_and_other_invalidates() {
        unimplemented!("not ported: MemoizingEvaluatorTest.removedNodeComesBackAndOtherInvalidates")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5586 MemoizingEvaluatorTest.removedInvalidatedNodeComesBackAndOtherInvalidates
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn removed_invalidated_node_comes_back_and_other_invalidates() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.removedInvalidatedNodeComesBackAndOtherInvalidates"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5624 MemoizingEvaluatorTest.cleanReverseDepFromDirtyNodeNotInBuild
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn clean_reverse_dep_from_dirty_node_not_in_build() {
        unimplemented!("not ported: MemoizingEvaluatorTest.cleanReverseDepFromDirtyNodeNotInBuild")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5678 MemoizingEvaluatorTest.errorChanged
    #[test]
    #[ignore = "incremental: needs invalidation of changed nodes (dirty nodes, versions, change \
                pruning)"]
    fn error_changed() {
        unimplemented!("not ported: MemoizingEvaluatorTest.errorChanged")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5692 MemoizingEvaluatorTest.duplicateUnfinishedDeps_NoKeepGoing
    #[test]
    #[ignore = "needs the restart model: functions are not restarted in normal evaluation, a \
                request computes the dep"]
    fn duplicate_unfinished_deps_no_keep_going() {
        unimplemented!("not ported: MemoizingEvaluatorTest.duplicateUnfinishedDeps_NoKeepGoing")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5702 MemoizingEvaluatorTest.externalDep
    #[test]
    #[ignore = "Environment.dependOnFuture (external deps) is not ported"]
    fn external_dep() {
        unimplemented!("not ported: MemoizingEvaluatorTest.externalDep")
    }
}
