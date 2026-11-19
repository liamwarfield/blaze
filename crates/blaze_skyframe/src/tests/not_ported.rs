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

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:843 MemoizingEvaluatorTest.alreadyAnalyzedBadTarget
    #[test]
    #[ignore = "needs a graph listener (NotifyingHelper), which the graph here does not have"]
    fn already_analyzed_bad_target() {
        unimplemented!("not ported: MemoizingEvaluatorTest.alreadyAnalyzedBadTarget")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:875 MemoizingEvaluatorTest.receiverToldOfVerifiedValueDependingOnCycle
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn receiver_told_of_verified_value_depending_on_cycle() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.receiverToldOfVerifiedValueDependingOnCycle"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1096 MemoizingEvaluatorTest.changeCycle_NoKeepGoing
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn change_cycle_no_keep_going() {
        unimplemented!("not ported: MemoizingEvaluatorTest.changeCycle_NoKeepGoing")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1101 MemoizingEvaluatorTest.changeCycle_KeepGoing
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn change_cycle_keep_going() {
        unimplemented!("not ported: MemoizingEvaluatorTest.changeCycle_KeepGoing")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1180 MemoizingEvaluatorTest.cycleAndSelfEdgeWithDirtyValue
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn cycle_and_self_edge_with_dirty_value() {
        unimplemented!("not ported: MemoizingEvaluatorTest.cycleAndSelfEdgeWithDirtyValue")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1393 MemoizingEvaluatorTest.cycleAndErrorAndReady
    #[test]
    #[ignore = "needs a graph listener (NotifyingHelper), which the graph here does not have"]
    fn cycle_and_error_and_ready() {
        unimplemented!("not ported: MemoizingEvaluatorTest.cycleAndErrorAndReady")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1546 MemoizingEvaluatorTest.nodeInvalidatedThenDoubleCycle
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn node_invalidated_then_double_cycle() {
        unimplemented!("not ported: MemoizingEvaluatorTest.nodeInvalidatedThenDoubleCycle")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1708 MemoizingEvaluatorTest.incompleteDirectDepsAreClearedBeforeInvalidation
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn incomplete_direct_deps_are_cleared_before_invalidation() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.incompleteDirectDepsAreClearedBeforeInvalidation"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:1866 MemoizingEvaluatorTest.incompleteDirectDepsForDirtyValue
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn incomplete_direct_deps_for_dirty_value() {
        unimplemented!("not ported: MemoizingEvaluatorTest.incompleteDirectDepsForDirtyValue")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2340 MemoizingEvaluatorTest.dirtyChildEnqueuesParentDuringCheckDependencies_ThrowDoesntEnqueue
    #[test]
    #[ignore = "needs a graph listener (NotifyingHelper), which the graph here does not have"]
    fn dirty_child_enqueues_parent_during_check_dependencies_throw_doesnt_enqueue() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.dirtyChildEnqueuesParentDuringCheckDependencies_ThrowDoesntEnqueue"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2346 MemoizingEvaluatorTest.dirtyChildEnqueuesParentDuringCheckDependencies_NoThrow
    #[test]
    #[ignore = "needs a graph listener (NotifyingHelper), which the graph here does not have"]
    fn dirty_child_enqueues_parent_during_check_dependencies_no_throw() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.dirtyChildEnqueuesParentDuringCheckDependencies_NoThrow"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:2351 MemoizingEvaluatorTest.removeReverseDepFromRebuildingNode
    #[test]
    #[ignore = "needs a graph listener (NotifyingHelper), which the graph here does not have"]
    fn remove_reverse_dep_from_rebuilding_node() {
        unimplemented!("not ported: MemoizingEvaluatorTest.removeReverseDepFromRebuildingNode")
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

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3478 MemoizingEvaluatorTest.dirtyAndChangedValueIsChanged
    #[test]
    #[ignore = "needs a graph listener (NotifyingHelper), which the graph here does not have"]
    fn dirty_and_changed_value_is_changed() {
        unimplemented!("not ported: MemoizingEvaluatorTest.dirtyAndChangedValueIsChanged")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3568 MemoizingEvaluatorTest.singleValueDependsOnManyDirtyValues
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn single_value_depends_on_many_dirty_values() {
        unimplemented!("not ported: MemoizingEvaluatorTest.singleValueDependsOnManyDirtyValues")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3708 MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithInvalidateRemoveError
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn dirty_children_properly_removed_with_invalidate_remove_error() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithInvalidateRemoveError"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3714 MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithInvalidateKeepError
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn dirty_children_properly_removed_with_invalidate_keep_error() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithInvalidateKeepError"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3720 MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithReevaluateRemoveError
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn dirty_children_properly_removed_with_reevaluate_remove_error() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithReevaluateRemoveError"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3726 MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithReevaluateKeepError
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn dirty_children_properly_removed_with_reevaluate_keep_error() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.dirtyChildrenProperlyRemovedWithReevaluateKeepError"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3816 MemoizingEvaluatorTest.manyDirtyValuesClearChildrenOnError
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn many_dirty_values_clear_children_on_error() {
        unimplemented!("not ported: MemoizingEvaluatorTest.manyDirtyValuesClearChildrenOnError")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:3825 MemoizingEvaluatorTest.manyDirtyValuesClearChildrenOnInterrupt
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn many_dirty_values_clear_children_on_interrupt() {
        unimplemented!("not ported: MemoizingEvaluatorTest.manyDirtyValuesClearChildrenOnInterrupt")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4140 MemoizingEvaluatorTest.changedBuildAfterFailedThenSuccessfulBuild
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn changed_build_after_failed_then_successful_build() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.changedBuildAfterFailedThenSuccessfulBuild"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4177 MemoizingEvaluatorTest.manyDirtyValuesClearChildrenOnSecondFail
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn many_dirty_values_clear_children_on_second_fail() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.manyDirtyValuesClearChildrenOnSecondFail"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4231 MemoizingEvaluatorTest.failedDirtyBuildInBuilder
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn failed_dirty_build_in_builder() {
        unimplemented!("not ported: MemoizingEvaluatorTest.failedDirtyBuildInBuilder")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4289 MemoizingEvaluatorTest.dirtyWithOwnErrorDependsOnTransientErrorTurningGood
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn dirty_with_own_error_depends_on_transient_error_turning_good() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.dirtyWithOwnErrorDependsOnTransientErrorTurningGood"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4394 MemoizingEvaluatorTest.dirtyWithRecoveryErrorDependsOnErrorTurningGood
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
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

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4712 MemoizingEvaluatorTest.changePruningWithIntermittentEvent
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn change_pruning_with_intermittent_event() {
        unimplemented!("not ported: MemoizingEvaluatorTest.changePruningWithIntermittentEvent")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4775 MemoizingEvaluatorTest.depEventPredicate
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn dep_event_predicate() {
        unimplemented!("not ported: MemoizingEvaluatorTest.depEventPredicate")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:4915 MemoizingEvaluatorTest.twoRailLeftRightDependenciesWithFailure
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn two_rail_left_right_dependencies_with_failure() {
        unimplemented!("not ported: MemoizingEvaluatorTest.twoRailLeftRightDependenciesWithFailure")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5205 MemoizingEvaluatorTest.shutDownBuildOnCachedError_Done
    #[test]
    #[ignore = "needs a graph listener (NotifyingHelper), which the graph here does not have"]
    fn shut_down_build_on_cached_error_done() {
        unimplemented!("not ported: MemoizingEvaluatorTest.shutDownBuildOnCachedError_Done")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5274 MemoizingEvaluatorTest.shutDownBuildOnCachedError_Verified
    #[test]
    #[ignore = "needs a graph listener (NotifyingHelper), which the graph here does not have"]
    fn shut_down_build_on_cached_error_verified() {
        unimplemented!("not ported: MemoizingEvaluatorTest.shutDownBuildOnCachedError_Verified")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5396 MemoizingEvaluatorTest.cachedErrorCausesRestart
    #[test]
    #[ignore = "needs a graph listener (NotifyingHelper), which the graph here does not have"]
    fn cached_error_causes_restart() {
        unimplemented!("not ported: MemoizingEvaluatorTest.cachedErrorCausesRestart")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5559 MemoizingEvaluatorTest.removedNodeComesBackAndInvalidates
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn removed_node_comes_back_and_invalidates() {
        unimplemented!("not ported: MemoizingEvaluatorTest.removedNodeComesBackAndInvalidates")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5572 MemoizingEvaluatorTest.removedNodeComesBackAndOtherInvalidates
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn removed_node_comes_back_and_other_invalidates() {
        unimplemented!("not ported: MemoizingEvaluatorTest.removedNodeComesBackAndOtherInvalidates")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5586 MemoizingEvaluatorTest.removedInvalidatedNodeComesBackAndOtherInvalidates
    #[test]
    #[ignore = "not ported yet: left for the next part of package I1 (the engine has incremental \
                evaluation, docs/skyframe-incremental.d/engine.md)"]
    fn removed_invalidated_node_comes_back_and_other_invalidates() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.removedInvalidatedNodeComesBackAndOtherInvalidates"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5624 MemoizingEvaluatorTest.cleanReverseDepFromDirtyNodeNotInBuild
    #[test]
    #[ignore = "needs a graph listener (NotifyingHelper), which the graph here does not have"]
    fn clean_reverse_dep_from_dirty_node_not_in_build() {
        unimplemented!("not ported: MemoizingEvaluatorTest.cleanReverseDepFromDirtyNodeNotInBuild")
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
