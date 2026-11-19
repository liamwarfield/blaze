//! The JUnit tests of lib/worker that are not ported yet: one ignored test per Java @Test
//! method (names kept), so that the counts show them. Ported test classes have their own
//! files (worker_*_test.rs).

mod worker_exec_root_test {
    // Java tests: src/test/java/com/google/devtools/build/lib/worker/WorkerExecRootTest.java
    #[test]
    #[ignore = "needs the SandboxHelper fixture of the Java test (not ported yet)"]
    fn clean_file_system() {}
    #[test]
    #[ignore = "needs the SandboxHelper fixture of the Java test (not ported yet)"]
    fn creates_and_cleans_input_symlinks() {}
    #[test]
    #[ignore = "needs the SandboxHelper fixture of the Java test (not ported yet)"]
    fn creates_output_dirs() {}
    #[test]
    #[ignore = "needs the SandboxHelper fixture of the Java test (not ported yet)"]
    fn workspace_files_are_not_deleted() {}
    #[test]
    #[ignore = "needs the SandboxHelper fixture of the Java test (not ported yet)"]
    fn recreates_empty_files() {}
    #[test]
    #[ignore = "needs the SandboxHelper fixture of the Java test (not ported yet)"]
    fn creates_and_deletes_sibling_external_repo_files() {}
}

mod worker_files_hash_test {
    // Java tests: src/test/java/com/google/devtools/build/lib/worker/WorkerFilesHashTest.java
    #[test]
    #[ignore = "needs a fake InputMetadataProvider with runfiles trees (not ported yet)"]
    fn get_worker_files_with_digests_returns_tools_with_correct_digests() {}
    #[test]
    #[ignore = "needs a fake InputMetadataProvider with runfiles trees (not ported yet)"]
    fn get_worker_files_with_digests_tree_artifact_tool_returns_expanded() {}
    #[test]
    #[ignore = "needs a fake InputMetadataProvider with runfiles trees (not ported yet)"]
    fn get_worker_files_with_digests_spawn_with_inputs_but_no_tools_returns_empty() {}
    #[test]
    #[ignore = "needs a fake InputMetadataProvider with runfiles trees (not ported yet)"]
    fn get_worker_files_with_digests_missing_digest_for_tool_fails() {}
    #[test]
    #[ignore = "needs a fake InputMetadataProvider with runfiles trees (not ported yet)"]
    fn get_worker_files_with_digests_io_exception_for_tool_metadata_fails() {}
}

mod worker_multiplexer_test {
    // Java tests: src/test/java/com/google/devtools/build/lib/worker/WorkerMultiplexerTest.java
    #[test]
    #[ignore = "needs a fake SubprocessFactory driving the multiplexer threads (not ported yet)"]
    fn test_get_response_no_outstanding_requests() {}
    #[test]
    #[ignore = "needs a fake SubprocessFactory driving the multiplexer threads (not ported yet)"]
    fn test_get_response_basic_concurrency() {}
    #[test]
    #[ignore = "needs a fake SubprocessFactory driving the multiplexer threads (not ported yet)"]
    fn test_get_response_slow_multiplexer() {}
    #[test]
    #[ignore = "needs a fake SubprocessFactory driving the multiplexer threads (not ported yet)"]
    fn test_get_response_slow_proxy() {}
    #[test]
    #[ignore = "needs a fake SubprocessFactory driving the multiplexer threads (not ported yet)"]
    fn work_dir_destroy_multiplexer_successfully_destroys_work_dir() {}
}

mod sandboxed_worker_proxy_test {
    // Java tests: src/test/java/com/google/devtools/build/lib/worker/SandboxedWorkerProxyTest.java
    #[test]
    #[ignore = "needs the SandboxHelper fixture and a fake multiplexer process (not ported yet)"]
    fn prepare_execution_creates_files_in_sandbox() {}
    #[test]
    #[ignore = "needs the SandboxHelper fixture and a fake multiplexer process (not ported yet)"]
    fn put_request_sets_sandbox_dir() {}
    #[test]
    #[ignore = "needs the SandboxHelper fixture and a fake multiplexer process (not ported yet)"]
    fn finish_execution_copies_outputs() {}
    #[test]
    #[ignore = "needs the SandboxHelper fixture and a fake multiplexer process (not ported yet)"]
    fn different_proxies_same_multiplexer_have_same_work_dir() {}
    #[test]
    #[ignore = "needs the SandboxHelper fixture and a fake multiplexer process (not ported yet)"]
    fn different_proxies_different_multiplexer_same_mnem_have_different_work_dirs() {}
}

mod worker_lifecycle_manager_test {
    // Java tests: src/test/java/com/google/devtools/build/lib/worker/WorkerLifecycleManagerTest.java
    #[test]
    #[ignore = "needs the Mockito fakes of WorkerPool and WorkerProcessMetricsCollector (not ported yet)"]
    fn test_evict_workers_do_nothing_low_memory_usage() {}
    #[test]
    #[ignore = "needs the Mockito fakes of WorkerPool and WorkerProcessMetricsCollector (not ported yet)"]
    fn test_evict_workers_do_nothing_zero_threshold() {}
    #[test]
    #[ignore = "needs the Mockito fakes of WorkerPool and WorkerProcessMetricsCollector (not ported yet)"]
    fn test_evict_workers_do_nothing_empty_metrics() {}
    #[test]
    #[ignore = "needs the Mockito fakes of WorkerPool and WorkerProcessMetricsCollector (not ported yet)"]
    fn test_get_eviction_candidates_select_only_worker() {}
    #[test]
    #[ignore = "needs the Mockito fakes of WorkerPool and WorkerProcessMetricsCollector (not ported yet)"]
    fn test_get_eviction_candidates_evict_largest_workers() {}
    #[test]
    #[ignore = "needs the Mockito fakes of WorkerPool and WorkerProcessMetricsCollector (not ported yet)"]
    fn test_get_eviction_candidates_number_of_workers_is_more_than_default_num_tests() {}
    #[test]
    #[ignore = "needs the Mockito fakes of WorkerPool and WorkerProcessMetricsCollector (not ported yet)"]
    fn test_get_eviction_candidates_evict_worker_with_same_menmonic_but_different_keys() {}
    #[test]
    #[ignore = "needs the Mockito fakes of WorkerPool and WorkerProcessMetricsCollector (not ported yet)"]
    fn test_get_eviction_candidates_evict_only_idle_workers() {}
    #[test]
    #[ignore = "needs the Mockito fakes of WorkerPool and WorkerProcessMetricsCollector (not ported yet)"]
    fn test_get_eviction_candidates_evict_different_worker_keys() {}
    #[test]
    #[ignore = "needs the Mockito fakes of WorkerPool and WorkerProcessMetricsCollector (not ported yet)"]
    fn test_get_eviction_candidates_test_doomed_workers() {}
    #[test]
    #[ignore = "needs the Mockito fakes of WorkerPool and WorkerProcessMetricsCollector (not ported yet)"]
    fn test_get_eviction_candidates_test_doomed_and_idle_workers() {}
    #[test]
    #[ignore = "needs the Mockito fakes of WorkerPool and WorkerProcessMetricsCollector (not ported yet)"]
    fn evict_workers_test_multiplex_workers() {}
    #[test]
    #[ignore = "needs the Mockito fakes of WorkerPool and WorkerProcessMetricsCollector (not ported yet)"]
    fn evict_workers_doom_multiplex_worker() {}
}

mod worker_process_metrics_collector_test {
    // Java tests: src/test/java/com/google/devtools/build/lib/worker/WorkerProcessMetricsCollectorTest.java
    #[test]
    #[ignore = "needs fakes of PsInfoCollector and CgroupsInfoCollector (not ported yet)"]
    fn test_register_worker_insert_different() {}
    #[test]
    #[ignore = "needs fakes of PsInfoCollector and CgroupsInfoCollector (not ported yet)"]
    fn test_register_worker_insert_multiplex() {}
    #[test]
    #[ignore = "needs fakes of PsInfoCollector and CgroupsInfoCollector (not ported yet)"]
    fn test_register_worker_insert_same() {}
    #[test]
    #[ignore = "needs fakes of PsInfoCollector and CgroupsInfoCollector (not ported yet)"]
    fn test_collect_metrics() {}
    #[test]
    #[ignore = "needs fakes of PsInfoCollector and CgroupsInfoCollector (not ported yet)"]
    fn test_collect_resource_usage_windows() {}
    #[test]
    #[ignore = "needs fakes of PsInfoCollector and CgroupsInfoCollector (not ported yet)"]
    fn test_collect_resource_usage_darwin_using_ps() {}
    #[test]
    #[ignore = "needs fakes of PsInfoCollector and CgroupsInfoCollector (not ported yet)"]
    fn test_collect_resource_usage_linux_using_ps() {}
    #[test]
    #[ignore = "needs fakes of PsInfoCollector and CgroupsInfoCollector (not ported yet)"]
    fn test_collect_resource_usage_linux_using_cgroups() {}
    #[test]
    #[ignore = "needs fakes of PsInfoCollector and CgroupsInfoCollector (not ported yet)"]
    fn test_worker_metrics_publish_comparator_compare() {}
    #[test]
    #[ignore = "needs fakes of PsInfoCollector and CgroupsInfoCollector (not ported yet)"]
    fn test_limit_worker_metrics_to_publish() {}
}

mod worker_spawn_runner_test {
    // Java tests: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java
    #[test]
    #[ignore = "needs the Mockito fakes of SpawnExecutionContext, ResourceManager and Worker (not ported yet)"]
    fn test_exec_in_worker_happy_path() {}
    #[test]
    #[ignore = "needs the Mockito fakes of SpawnExecutionContext, ResourceManager and Worker (not ported yet)"]
    fn test_exec_in_worker_virtual_inputs_doesnt_query_input_file_cache() {}
    #[test]
    #[ignore = "needs the Mockito fakes of SpawnExecutionContext, ResourceManager and Worker (not ported yet)"]
    fn test_exec_in_worker_finishes_async_on_interrupt() {}
    #[test]
    #[ignore = "needs the Mockito fakes of SpawnExecutionContext, ResourceManager and Worker (not ported yet)"]
    fn test_exec_in_worker_sends_cancel_message_on_interrupt() {}
    #[test]
    #[ignore = "needs the Mockito fakes of SpawnExecutionContext, ResourceManager and Worker (not ported yet)"]
    fn test_exec_in_worker_unsandboxed_dies_on_interrupt() {}
    #[test]
    #[ignore = "needs the Mockito fakes of SpawnExecutionContext, ResourceManager and Worker (not ported yet)"]
    fn test_exec_in_worker_no_multiplex_with_dynamic() {}
    #[test]
    #[ignore = "needs the Mockito fakes of SpawnExecutionContext, ResourceManager and Worker (not ported yet)"]
    fn test_exec_in_worker_shows_log_file_in_exception() {}
    #[test]
    #[ignore = "needs the Mockito fakes of SpawnExecutionContext, ResourceManager and Worker (not ported yet)"]
    fn test_exec_in_worker_throws_with_empty_response() {}
    #[test]
    #[ignore = "needs the Mockito fakes of SpawnExecutionContext, ResourceManager and Worker (not ported yet)"]
    fn test_expand_argument_expands_arguments_recursively() {}
    #[test]
    #[ignore = "needs the Mockito fakes of SpawnExecutionContext, ResourceManager and Worker (not ported yet)"]
    fn test_expand_argument_expands_only_proper_arguments() {}
    #[test]
    #[ignore = "needs the Mockito fakes of SpawnExecutionContext, ResourceManager and Worker (not ported yet)"]
    fn test_expand_argument_fails_on_missing_file() {}
    #[test]
    #[ignore = "needs the Mockito fakes of SpawnExecutionContext, ResourceManager and Worker (not ported yet)"]
    fn test_can_exec_checks_requirements() {}
    #[test]
    #[ignore = "needs the Mockito fakes of SpawnExecutionContext, ResourceManager and Worker (not ported yet)"]
    fn test_can_exec_obeys_allowlist() {}
    #[test]
    #[ignore = "needs the Mockito fakes of SpawnExecutionContext, ResourceManager and Worker (not ported yet)"]
    fn test_expand_argument_fails_on_undeclared_input() {}
}

mod worker_module_test {
    // Java tests: src/test/java/com/google/devtools/build/lib/worker/WorkerModuleTest.java
    #[test]
    #[ignore = "needs a CommandEnvironment and BuildStartingEvent fakes (not ported yet)"]
    fn build_starting_creates_pools() {}
    #[test]
    #[ignore = "needs a CommandEnvironment and BuildStartingEvent fakes (not ported yet)"]
    fn build_starting_no_restart_on_sandbox_change() {}
    #[test]
    #[ignore = "needs a CommandEnvironment and BuildStartingEvent fakes (not ported yet)"]
    fn build_starting_restarts_on_outputbase_changes() {}
    #[test]
    #[ignore = "needs a CommandEnvironment and BuildStartingEvent fakes (not ported yet)"]
    fn build_starting_restarts_on_use_cgroups_on_linux_changes() {}
    #[test]
    #[ignore = "needs a CommandEnvironment and BuildStartingEvent fakes (not ported yet)"]
    fn build_starting_clears_logs_on_factory_creation() {}
    #[test]
    #[ignore = "needs a CommandEnvironment and BuildStartingEvent fakes (not ported yet)"]
    fn build_starting_restarts_on_num_multiplex_workers_changes() {}
    #[test]
    #[ignore = "needs a CommandEnvironment and BuildStartingEvent fakes (not ported yet)"]
    fn build_starting_restarts_on_num_workers_changes() {}
    #[test]
    #[ignore = "needs a CommandEnvironment and BuildStartingEvent fakes (not ported yet)"]
    fn build_starting_survives_no_worker_dir() {}
    #[test]
    #[ignore = "needs a CommandEnvironment and BuildStartingEvent fakes (not ported yet)"]
    fn build_starting_cleans_stale_trash_dir_cleaned_on_first_build() {}
}

mod worker_spawn_strategy_test {
    // Java tests: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnStrategyTest.java
    #[test]
    #[ignore = "needs a fake SpawnRunner and ActionExecutionContext (not ported yet)"]
    fn expand_arguments_preserves_empty_lines() {}
}

mod work_request_handler_test {
    // Java tests: src/test/java/com/google/devtools/build/lib/worker/WorkRequestHandlerTest.java
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_normal_work_request() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_multiplex_work_request() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_multiplex_work_request_stops_threads_on_shutdown() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_multiplex_work_request_stops_worker_on_exception() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_output() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_exception() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_cancel_request_exactly_one_response_sent() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_cancel_request_sends_response_when_done() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_cancel_request_no_double_cancel_response() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_cancel_request_sends_no_response_when_already_sent() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_work_request_handler_with_work_request_callback() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_worker_io_does_wrap_system_streams() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_worker_io_does_capture_standard_out_and_error_streams() {}
}
