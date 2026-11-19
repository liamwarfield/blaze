//! Port of `src/test/java/com/google/devtools/build/lib/remote/RemoteModuleTest.java`.
//!
//! Only `computeActionExecutionSalt` is ported; the other tests drive `RemoteModule.beforeCommand`
//! through a `BlazeRuntime` with in-process gRPC servers and a channel factory the module takes
//! as a parameter, which `blaze_remote::remote_module` does not have (it creates its channels
//! itself; the binary is checked end to end against Bazel's remote worker, docs/remote.md).

use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_options::Options;
use blaze_remote::remote_module::compute_action_execution_salt;

// Java: RemoteModuleTest.parseRemoteOptions
fn parse_remote_options(args: &[&str]) -> RemoteOptions {
    Options::<RemoteOptions>::parse(args)
        .unwrap_or_else(|e| panic!("{}", e.get_message()))
        .into_options()
}

// Java: com/google/devtools/build/lib/remote/RemoteModuleTest.java:877 RemoteModuleTest.computeActionExecutionSalt_remoteEndpointOrInstanceChanges_doesNotChange
#[test]
fn compute_action_execution_salt_remote_endpoint_or_instance_changes_does_not_change() {
    let base_options = parse_remote_options(&[
        "--remote_cache=grpc://cache1",
        "--remote_executor=grpc://exec1",
        "--remote_instance_name=instance1",
    ]);
    let base_salt = compute_action_execution_salt(Some(&base_options));
    assert_ne!(compute_action_execution_salt(None), base_salt);
    assert_ne!(
        compute_action_execution_salt(Some(&parse_remote_options(&[]))),
        base_salt
    );
    assert_ne!(
        compute_action_execution_salt(Some(&parse_remote_options(&[
            "--remote_cache=grpc://cache1",
            "--remote_default_exec_properties=OSFamily=linux",
        ]))),
        base_salt
    );

    let changed_instance_name = parse_remote_options(&[
        "--remote_cache=grpc://cache1",
        "--remote_executor=grpc://exec1",
        "--remote_instance_name=instance2",
    ]);
    assert_eq!(
        compute_action_execution_salt(Some(&changed_instance_name)),
        base_salt
    );

    let changed_remote_cache = parse_remote_options(&[
        "--remote_cache=grpc://cache2",
        "--remote_executor=grpc://exec1",
        "--remote_instance_name=instance1",
    ]);
    assert_eq!(
        compute_action_execution_salt(Some(&changed_remote_cache)),
        base_salt
    );

    let changed_remote_executor = parse_remote_options(&[
        "--remote_cache=grpc://cache1",
        "--remote_executor=grpc://exec2",
        "--remote_instance_name=instance1",
    ]);
    assert_eq!(
        compute_action_execution_salt(Some(&changed_remote_executor)),
        base_salt
    );

    let changed_remote_downloader = parse_remote_options(&[
        "--remote_cache=grpc://cache1",
        "--remote_executor=grpc://exec1",
        "--remote_instance_name=instance1",
        "--experimental_remote_downloader=grpc://downloader1",
    ]);
    assert_eq!(
        compute_action_execution_salt(Some(&changed_remote_downloader)),
        base_salt
    );

    let changed_bytestream_prefix = parse_remote_options(&[
        "--remote_cache=grpc://cache1",
        "--remote_executor=grpc://exec1",
        "--remote_instance_name=instance1",
        "--remote_bytestream_uri_prefix=host/instance2",
    ]);
    assert_eq!(
        compute_action_execution_salt(Some(&changed_bytestream_prefix)),
        base_salt
    );

    let changed_remote_proxy = parse_remote_options(&[
        "--remote_cache=grpc://cache1",
        "--remote_executor=grpc://exec1",
        "--remote_instance_name=instance1",
        "--remote_proxy=unix:/socket2",
    ]);
    assert_eq!(
        compute_action_execution_salt(Some(&changed_remote_proxy)),
        base_salt
    );
}

#[test]
#[ignore = "needs RemoteModule.beforeCommand with an injected channel factory and in-process gRPC servers (BlazeRuntime harness)"]
fn remote_grpc_service_config_passes_remote_timeout_config_to_channel_factory() {}

#[test]
#[ignore = "needs RemoteModule.beforeCommand with an injected channel factory and in-process gRPC servers (BlazeRuntime harness)"]
fn remote_grpc_service_config_passes_user_supplied_json_file_to_channel_factory() {}

#[test]
#[ignore = "needs RemoteModule.beforeCommand with an injected channel factory and in-process gRPC servers (BlazeRuntime harness)"]
fn test_verify_capabilities_none() {}

#[test]
#[ignore = "needs RemoteModule.beforeCommand with an injected channel factory and in-process gRPC servers (BlazeRuntime harness)"]
fn test_verify_capabilities_execution_and_cache_for_single_endpoint() {}

#[test]
#[ignore = "needs RemoteModule.beforeCommand with an injected channel factory and in-process gRPC servers (BlazeRuntime harness)"]
fn test_verify_capabilities_cache_only_endpoint() {}

#[test]
#[ignore = "needs RemoteModule.beforeCommand with an injected channel factory and in-process gRPC servers (BlazeRuntime harness)"]
fn test_verify_capabilities_execution_and_cache_for_different_endpoints() {}

#[test]
#[ignore = "needs RemoteModule.beforeCommand with an injected channel factory and in-process gRPC servers (BlazeRuntime harness)"]
fn test_verify_capabilities_execution_only_and_cache_only_endpoints() {}

#[test]
#[ignore = "needs RemoteModule.beforeCommand with an injected channel factory and in-process gRPC servers (BlazeRuntime harness)"]
fn test_netrc_netrc_without_remote_cache() {}

#[test]
#[ignore = "needs RemoteModule.beforeCommand with an injected channel factory and in-process gRPC servers (BlazeRuntime harness)"]
fn test_cache_capabilities_propagated_to_remote_cache() {}

#[test]
#[ignore = "needs RemoteModule.beforeCommand with an injected channel factory and in-process gRPC servers (BlazeRuntime harness)"]
fn test_cache_capabilities_propagated_to_remote_execution_cache() {}

#[test]
#[ignore = "needs RemoteModule.beforeCommand with an injected channel factory and in-process gRPC servers (BlazeRuntime harness)"]
fn test_verify_capabilities_execution_and_cache_for_single_endpoint_with_circuit_breaker() {}

#[test]
#[ignore = "needs RemoteModule.beforeCommand with an injected channel factory and in-process gRPC servers (BlazeRuntime harness)"]
fn test_verify_capabilities_cache_only_endpoint_with_circuit_breaker() {}

#[test]
#[ignore = "BazelOutputService (--experimental_remote_output_service) is not ported"]
fn bazel_output_service_no_remote_cache_exit() {}

#[test]
#[ignore = "DiskCacheGarbageCollectorIdleTask: Blaze has no idle server tasks"]
fn disk_cache_garbage_collection_idle_task_disabled() {}

#[test]
#[ignore = "DiskCacheGarbageCollectorIdleTask: Blaze has no idle server tasks"]
fn disk_cache_garbage_collection_idle_task_enabled() {}

#[test]
#[ignore = "the repository remote helpers factory (initRepoHelpersAndOverlayFs) is not ported"]
fn repository_remote_helpers_factory_initialized_for_disk_cache_only_path() {}

#[test]
#[ignore = "the remote downloader (--experimental_remote_downloader) is not ported"]
fn remote_downloader_separate_endpoint_channels_are_closed_after_command() {}

#[test]
#[ignore = "the remote downloader (--experimental_remote_downloader) is not ported"]
fn remote_downloader_shared_endpoint_channel_is_closed_once_cache_is_released() {}

#[test]
#[ignore = "RemoteModule's executor service pool (one tokio runtime in Blaze)"]
fn executor_service_without_build_request_options_preserves_pool_size() {}
