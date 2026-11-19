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
    #[ignore = "SkyFunctionException.validateExceptionType checks the Java exception classes given to \
                getValueOrThrow (a supertype of RuntimeException is an IllegalStateException); \
                get_value_or_throw takes one concrete Rust error type, there is no class to check"]
    fn validate_exception_type_in_different_position() {
        unimplemented!("not ported: ParallelEvaluatorTest.validateExceptionTypeInDifferentPosition")
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:2527 ParallelEvaluatorTest.validateExceptionTypeWithDifferentException
    #[test]
    #[ignore = "SkyFunctionException.validateExceptionType checks the Java exception classes given to \
                getValueOrThrow (Exception, NullPointerException, InterruptedException); \
                get_value_or_throw takes one concrete Rust error type, there is no class to check"]
    fn validate_exception_type_with_different_exception() {
        unimplemented!(
            "not ported: ParallelEvaluatorTest.validateExceptionTypeWithDifferentException"
        )
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
    #[ignore = "Environment.getParallelEvaluationExecutor and an injected QuiescingExecutor \
                (ParallelEvaluator's executor parameter) are not ported: the blocking engine has \
                no executor a function can submit to"]
    fn test_parallel_sky_function_computation_runnables_on_both_current_and_external_threads() {
        unimplemented!(
            "not ported: ParallelEvaluatorTest.testParallelSkyFunctionComputation_runnablesOnBothCurrentAndExternalThreads"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:4206 ParallelEvaluatorTest.testParallelSkyFunctionComputation_runnablesOnExternalThreadsOnly
    #[test]
    #[ignore = "Environment.getParallelEvaluationExecutor and an injected QuiescingExecutor \
                (ParallelEvaluator's executor parameter) are not ported: the blocking engine has \
                no executor a function can submit to"]
    fn test_parallel_sky_function_computation_runnables_on_external_threads_only() {
        unimplemented!(
            "not ported: ParallelEvaluatorTest.testParallelSkyFunctionComputation_runnablesOnExternalThreadsOnly"
        )
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/ParallelEvaluatorTest.java:4283 ParallelEvaluatorTest.customKeepGoingPredicate_differentPerKey
    #[test]
    #[ignore = "the per-key keep_going predicate (ParallelEvaluatorContext.keepGoing(SkyKey)) is not \
                ported (lib.rs `Not ported`): keep_going is one flag per evaluation"]
    fn custom_keep_going_predicate_different_per_key() {
        unimplemented!("not ported: ParallelEvaluatorTest.customKeepGoingPredicate_differentPerKey")
    }
}

mod memoizing_evaluator_test {
    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:186 MemoizingEvaluatorTest.injectGraphTransformer_transformedGraphUsedForInMemoryGraph
    #[test]
    #[ignore = "the graph is the concrete InMemoryGraphImpl; injectGraphTransformerForTesting is \
                ported as a listener hook on it (graph_listener.rs), so getInMemoryGraph cannot \
                return a substituted (mock) graph"]
    fn inject_graph_transformer_transformed_graph_used_for_in_memory_graph() {
        unimplemented!(
            "not ported: MemoizingEvaluatorTest.injectGraphTransformer_transformedGraphUsedForInMemoryGraph"
        )
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

    // Java: src/test/java/com/google/devtools/build/skyframe/MemoizingEvaluatorTest.java:5702 MemoizingEvaluatorTest.externalDep
    #[test]
    #[ignore = "Environment.dependOnFuture (external deps) is not ported"]
    fn external_dep() {
        unimplemented!("not ported: MemoizingEvaluatorTest.externalDep")
    }
}
