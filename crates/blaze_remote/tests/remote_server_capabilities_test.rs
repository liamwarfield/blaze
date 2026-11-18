#![cfg(feature = "wip")]
// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of `src/test/java/com/google/devtools/build/lib/remote/RemoteServerCapabilitiesTest.java`.
//!
//! Java runs the fake server in process (`InProcessServerBuilder`); the port's fake server
//! listens on localhost (`util::grpc_server`). Java's `ServerInterceptor`s that validate the
//! request headers are assertions at the start of the service method, which sees the headers.

mod util;

use std::io;
use std::sync::Arc;

use async_trait::async_trait;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_remote::api_version::ApiVersion;
use blaze_remote::authandtls::new_exec_headers_interceptor;
use blaze_remote::common::errors;
use blaze_remote::grpc::ClientInterceptor;
use blaze_remote::reapi::priority_capabilities::PriorityRange;
use blaze_remote::reapi::{
    ActionCacheUpdateCapabilities, CacheCapabilities, ExecutionCapabilities,
    GetCapabilitiesRequest, PriorityCapabilities, RequestMetadata, ServerCapabilities,
    digest_function,
};
use blaze_remote::remote_retrier::{
    EXPERIMENTAL_GRPC_RESULT_CLASSIFIER, ExponentialBackoff, RemoteRetrier,
};
use blaze_remote::remote_server_capabilities::{
    RemoteServerCapabilities, ServerCapabilitiesRequirement, check_client_server_compatibility,
};
use blaze_remote::retrier::{
    ALLOW_ALL_CALLS, Backoff, BackoffSupplier, RejectedExecutionException, ResultClassifier,
    ResultClassifierResult, RetryScheduler,
};
use blaze_remote::runtime::get_from_future;
use blaze_remote::util::tracing_metadata_utils;
use parking_lot::Mutex;
use prost::Message as _;
use regex::Regex;

use util::grpc_server::{FakeCapabilities, FakeServer};

/// `TestUtils.ZeroDelayListeningScheduledExecutorService`: schedules all tasks with zero delay.
// Java: com/google/devtools/build/lib/remote/util/TestUtils.java:69 TestUtils.ZeroDelayListeningScheduledExecutorService
struct ZeroDelayRetryScheduler;

#[async_trait]
impl RetryScheduler for ZeroDelayRetryScheduler {
    async fn schedule(&self, _delay_millis: i64) -> Result<(), RejectedExecutionException> {
        Ok(())
    }
}

/// `TestUtils.newRemoteRetrier(backoff, resultClassifier, retryScheduler)`.
// Java: com/google/devtools/build/lib/remote/util/TestUtils.java:46 TestUtils.newRemoteRetrier
fn new_remote_retrier(
    backoff: BackoffSupplier,
    result_classifier: Arc<dyn ResultClassifier>,
) -> Arc<RemoteRetrier> {
    Arc::new(RemoteRetrier::with_sleeper(
        backoff,
        Arc::new(move |e: &io::Error| {
            if errors::status_from_throwable(e).map(|s| s.code()) == Some(tonic::Code::Cancelled) {
                ResultClassifierResult::Success
            } else {
                result_classifier.test(e)
            }
        }),
        Arc::new(ZeroDelayRetryScheduler),
        ALLOW_ALL_CALLS.clone(),
        Arc::new(|_millis: i64| -> io::Result<()> {
            /* don't wait in tests */
            Ok(())
        }),
    ))
}

/// `() -> new ExponentialBackoff(remoteOptions)`.
fn exponential_backoff(remote_options: &RemoteOptions) -> BackoffSupplier {
    let remote_options = remote_options.clone();
    Arc::new(move || -> Arc<dyn Backoff> {
        Arc::new(ExponentialBackoff::from_options(&remote_options))
    })
}

/// The fields of the Java test class, as `setUp` leaves them.
struct Fixture {
    fake_server: FakeServer,
}

impl Fixture {
    // Java: com/google/devtools/build/lib/remote/RemoteServerCapabilitiesTest.java:67 RemoteServerCapabilitiesTest.setUp
    fn set_up() -> Fixture {
        // PORT: Java reads the version from BlazeVersionInfo.
        tracing_metadata_utils::set_tool_version("test");
        // Use a mutable service registry for later registering the service impl for each test
        // case.
        Fixture {
            fake_server: FakeServer::new(),
        }
    }
}

/// `matcher.find()` of `pattern` in `s` (Truth's `containsMatch`).
fn contains_match(s: &str, pattern: &str) -> bool {
    Regex::new(pattern).unwrap().is_match(s)
}

/// Capture the request headers from a client. Useful for testing metadata propagation.
// Java: com/google/devtools/build/lib/remote/RemoteServerCapabilitiesTest.java:90 RemoteServerCapabilitiesTest.RequestHeadersValidator
fn request_headers_validator<T>(request: &tonic::Request<T>) {
    let meta = request
        .metadata()
        .get_bin(tracing_metadata_utils::METADATA_KEY)
        .expect("no RequestMetadata header");
    let meta = RequestMetadata::decode(meta.to_bytes().unwrap()).unwrap();
    assert_eq!(meta.correlated_invocations_id, "build-req-id");
    assert_eq!(meta.tool_invocation_id, "command-id");
    assert!(!meta.action_id.is_empty());
    let tool_details = meta.tool_details.unwrap_or_default();
    assert_eq!(tool_details.tool_name, "bazel");
    // BlazeVersionInfo.instance().getVersion()
    assert_eq!(tool_details.tool_version, "test");
}

// Java: com/google/devtools/build/lib/remote/RemoteServerCapabilitiesTest.java:106 RemoteServerCapabilitiesTest.RequestCustomHeadersValidator
fn request_custom_headers_validator<T>(request: &tonic::Request<T>) {
    let headers = request.metadata();
    assert_eq!(
        headers.get("key1").map(|v| v.to_str().unwrap()),
        Some("Value1")
    );
    assert_eq!(
        headers.get("key2").map(|v| v.to_str().unwrap()),
        Some("Value2")
    );
}

#[test]
fn test_custom_headers_are_attached() {
    let mut f = Fixture::set_up();
    let caps = ServerCapabilities {
        execution_capabilities: Some(ExecutionCapabilities {
            exec_enabled: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let server_caps = caps.clone();
    f.fake_server
        .service_registry
        .add_capabilities(FakeCapabilities {
            get_capabilities: Some(Box::new(
                move |request: tonic::Request<GetCapabilitiesRequest>|
                      -> Result<ServerCapabilities, tonic::Status> {
                    request_custom_headers_validator(&request);
                    Ok(server_caps.clone())
                },
            )),
        });
    f.fake_server.start();

    let mut remote_options = RemoteOptions::default();
    remote_options.remote_headers = vec![
        ("Key1".to_string(), "Value1".to_string()),
        ("Key2".to_string(), "Value2".to_string()),
    ];

    let retrier = new_remote_retrier(
        exponential_backoff(&remote_options),
        EXPERIMENTAL_GRPC_RESULT_CLASSIFIER.clone(),
    );
    let interceptors: Vec<Arc<dyn ClientInterceptor>> =
        vec![Arc::new(new_exec_headers_interceptor(&remote_options))];
    let channel = f.fake_server.channel(interceptors, None);
    let client = RemoteServerCapabilities::new(
        "build-req-id",
        "command-id",
        Some("instance"),
        None,
        retrier,
    );

    assert_eq!(get_from_future(client.get(&channel)).unwrap(), caps);
}

#[test]
fn test_get_capabilities_with_retries() {
    let mut f = Fixture::set_up();
    let caps = ServerCapabilities {
        execution_capabilities: Some(ExecutionCapabilities {
            exec_enabled: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let server_caps = caps.clone();
    const MAX_ERRORS: i32 = 3;
    let num_errors = Mutex::new(0);
    f.fake_server
        .service_registry
        .add_capabilities(FakeCapabilities {
            get_capabilities: Some(Box::new(
                move |request: tonic::Request<GetCapabilitiesRequest>|
                      -> Result<ServerCapabilities, tonic::Status> {
                    request_headers_validator(&request);
                    let mut num_errors = num_errors.lock();
                    if *num_errors < MAX_ERRORS {
                        *num_errors += 1;
                        Err(tonic::Status::unavailable("")) // Retriable error.
                    } else {
                        Ok(server_caps.clone())
                    }
                },
            )),
        });
    f.fake_server.start();

    let remote_options = RemoteOptions::default();
    let retrier = new_remote_retrier(
        exponential_backoff(&remote_options),
        EXPERIMENTAL_GRPC_RESULT_CLASSIFIER.clone(),
    );
    let channel = f.fake_server.channel(vec![], None);
    let client = RemoteServerCapabilities::new(
        "build-req-id",
        "command-id",
        Some("instance"),
        /* callCredentials= */ None,
        retrier,
    );

    assert_eq!(get_from_future(client.get(&channel)).unwrap(), caps);
}

#[test]
fn test_check_client_server_compatibility_no_checks() {
    let st = check_client_server_compatibility(
        &ServerCapabilities::default(),
        &RemoteOptions::default(),
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::None,
    );
    assert!(st.is_ok());
}

#[test]
fn test_check_client_server_compatibility_api_version_deprecated() {
    let caps = ServerCapabilities {
        deprecated_api_version: Some(ApiVersion::LOW.to_sem_ver()),
        low_api_version: Some(ApiVersion::new(100, 0, 0, "").to_sem_ver()),
        high_api_version: Some(ApiVersion::new(100, 0, 0, "").to_sem_ver()),
        cache_capabilities: Some(CacheCapabilities {
            digest_functions: vec![digest_function::Value::Sha256 as i32],
            action_cache_update_capabilities: Some(ActionCacheUpdateCapabilities {
                update_enabled: true,
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut remote_options = RemoteOptions::default();
    remote_options.remote_cache = Some("server:port".to_string());
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::Cache,
    );
    assert!(st.get_errors().is_empty());
    assert_eq!(st.get_warnings().len(), 1);
    assert!(contains_match(
        &st.get_warnings()[0],
        "API.*deprecated.*100.0"
    ));
}

#[test]
fn test_check_client_server_compatibility_api_version_unsupported() {
    let caps = ServerCapabilities {
        low_api_version: Some(ApiVersion::new(100, 0, 0, "").to_sem_ver()),
        high_api_version: Some(ApiVersion::new(100, 0, 0, "").to_sem_ver()),
        cache_capabilities: Some(CacheCapabilities {
            digest_functions: vec![digest_function::Value::Sha256 as i32],
            action_cache_update_capabilities: Some(ActionCacheUpdateCapabilities {
                update_enabled: true,
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut remote_options = RemoteOptions::default();
    remote_options.remote_cache = Some("server:port".to_string());
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::Cache,
    );
    assert_eq!(st.get_errors().len(), 1);
    assert!(contains_match(
        &st.get_errors()[0],
        "API.*not supported.*100.0"
    ));
}

#[test]
fn test_check_client_server_compatibility_remote_cache_does_not_support_digest_function() {
    let caps = ServerCapabilities {
        low_api_version: Some(ApiVersion::LOW.to_sem_ver()),
        high_api_version: Some(ApiVersion::HIGH.to_sem_ver()),
        cache_capabilities: Some(CacheCapabilities {
            digest_functions: vec![digest_function::Value::Md5 as i32],
            action_cache_update_capabilities: Some(ActionCacheUpdateCapabilities {
                update_enabled: true,
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut remote_options = RemoteOptions::default();
    remote_options.remote_cache = Some("server:port".to_string());
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::Cache,
    );
    assert_eq!(st.get_errors().len(), 1);
    assert!(contains_match(
        &st.get_errors()[0],
        "Cannot use hash function"
    ));
}

#[test]
fn test_check_client_server_compatibility_remote_cache_does_not_support_update() {
    let caps = ServerCapabilities {
        low_api_version: Some(ApiVersion::LOW.to_sem_ver()),
        high_api_version: Some(ApiVersion::HIGH.to_sem_ver()),
        cache_capabilities: Some(CacheCapabilities {
            digest_functions: vec![digest_function::Value::Sha256 as i32],
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut remote_options = RemoteOptions::default();
    remote_options.remote_cache = Some("server:port".to_string());
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::Cache,
    );
    assert!(st.get_errors().is_empty());
    assert_eq!(st.get_warnings().len(), 1);
    assert!(
        st.get_warnings()[0].contains("remote cache does not support uploading action results")
    );

    // Ignored when no local upload.
    remote_options.remote_upload_local_results = false;
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::Cache,
    );
    assert!(st.is_ok());
}

#[test]
fn test_check_client_server_compatibility_remote_execution_is_disabled() {
    let caps = ServerCapabilities {
        low_api_version: Some(ApiVersion::LOW.to_sem_ver()),
        high_api_version: Some(ApiVersion::HIGH.to_sem_ver()),
        cache_capabilities: Some(CacheCapabilities {
            digest_functions: vec![digest_function::Value::Sha256 as i32],
            action_cache_update_capabilities: Some(ActionCacheUpdateCapabilities {
                update_enabled: true,
            }),
            ..Default::default()
        }),
        execution_capabilities: Some(ExecutionCapabilities {
            digest_function: digest_function::Value::Sha256 as i32,
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut remote_options = RemoteOptions::default();
    remote_options.remote_executor = Some("server:port".to_string());
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::ExecutionAndCache,
    );
    assert_eq!(st.get_errors().len(), 1);
    assert!(contains_match(
        &st.get_errors()[0],
        "Remote execution is not supported"
    ));
    assert!(contains_match(
        &st.get_errors()[0],
        "not authorized to use remote execution"
    ));
}

#[test]
fn test_check_client_server_compatibility_remote_execution_does_not_support_digest_function() {
    let caps = ServerCapabilities {
        low_api_version: Some(ApiVersion::LOW.to_sem_ver()),
        high_api_version: Some(ApiVersion::HIGH.to_sem_ver()),
        cache_capabilities: Some(CacheCapabilities {
            digest_functions: vec![digest_function::Value::Sha256 as i32],
            action_cache_update_capabilities: Some(ActionCacheUpdateCapabilities {
                update_enabled: true,
            }),
            ..Default::default()
        }),
        execution_capabilities: Some(ExecutionCapabilities {
            digest_function: digest_function::Value::Md5 as i32,
            exec_enabled: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut remote_options = RemoteOptions::default();
    remote_options.remote_executor = Some("server:port".to_string());
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::ExecutionAndCache,
    );
    assert_eq!(st.get_errors().len(), 1);
    assert!(contains_match(
        &st.get_errors()[0],
        "Cannot use hash function"
    ));
}

#[test]
fn test_check_client_server_compatibility_local_fallback_no_remote_cache_update() {
    let caps = ServerCapabilities {
        low_api_version: Some(ApiVersion::LOW.to_sem_ver()),
        high_api_version: Some(ApiVersion::HIGH.to_sem_ver()),
        cache_capabilities: Some(CacheCapabilities {
            digest_functions: vec![digest_function::Value::Sha256 as i32],
            ..Default::default()
        }),
        execution_capabilities: Some(ExecutionCapabilities {
            digest_function: digest_function::Value::Sha256 as i32,
            exec_enabled: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut remote_options = RemoteOptions::default();
    remote_options.remote_executor = Some("server:port".to_string());
    remote_options.remote_local_fallback = true;
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::ExecutionAndCache,
    );
    assert!(st.get_errors().is_empty());
    assert_eq!(st.get_warnings().len(), 1);
    assert!(
        st.get_warnings()[0].contains("remote cache does not support uploading action results")
    );

    // Ignored when no fallback.
    remote_options.remote_local_fallback = false;
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::ExecutionAndCache,
    );
    assert!(st.get_errors().is_empty());
    assert_eq!(st.get_warnings().len(), 1);
    assert!(
        st.get_warnings()[0].contains("remote cache does not support uploading action results")
    );

    // Ignored when no uploading local results.
    remote_options.remote_local_fallback = true;
    remote_options.remote_upload_local_results = false;
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::ExecutionAndCache,
    );
    assert!(st.is_ok());
}

/// The priority ranges 1-2 and 5-10.
fn priority_capabilities() -> PriorityCapabilities {
    PriorityCapabilities {
        priorities: vec![
            PriorityRange {
                min_priority: 1,
                max_priority: 2,
            },
            PriorityRange {
                min_priority: 5,
                max_priority: 10,
            },
        ],
    }
}

#[test]
fn test_check_client_server_compatibility_cache_priority() {
    let caps = ServerCapabilities {
        low_api_version: Some(ApiVersion::LOW.to_sem_ver()),
        high_api_version: Some(ApiVersion::HIGH.to_sem_ver()),
        cache_capabilities: Some(CacheCapabilities {
            digest_functions: vec![digest_function::Value::Sha256 as i32],
            cache_priority_capabilities: Some(priority_capabilities()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut remote_options = RemoteOptions::default();
    remote_options.remote_cache = Some("server:port".to_string());
    remote_options.remote_upload_local_results = false;
    remote_options.remote_result_cache_priority = 11;
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::Cache,
    );
    assert_eq!(st.get_errors().len(), 1);
    assert!(contains_match(
        &st.get_errors()[0],
        "remote_result_cache_priority"
    ));

    // Valid value in range.
    remote_options.remote_result_cache_priority = 10;
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::Cache,
    );
    assert!(st.is_ok());

    // Check not performed if the value is 0.
    remote_options.remote_result_cache_priority = 0;
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::Cache,
    );
    assert!(st.is_ok());
}

#[test]
fn test_check_client_server_compatibility_execution_priority() {
    let caps = ServerCapabilities {
        low_api_version: Some(ApiVersion::LOW.to_sem_ver()),
        high_api_version: Some(ApiVersion::HIGH.to_sem_ver()),
        cache_capabilities: Some(CacheCapabilities {
            digest_functions: vec![digest_function::Value::Sha256 as i32],
            ..Default::default()
        }),
        execution_capabilities: Some(ExecutionCapabilities {
            digest_function: digest_function::Value::Sha256 as i32,
            exec_enabled: true,
            execution_priority_capabilities: Some(priority_capabilities()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut remote_options = RemoteOptions::default();
    remote_options.remote_executor = Some("server:port".to_string());
    remote_options.remote_upload_local_results = false;
    remote_options.remote_execution_priority = 11;
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::ExecutionAndCache,
    );
    assert_eq!(st.get_errors().len(), 1);
    assert!(contains_match(
        &st.get_errors()[0],
        "remote_execution_priority"
    ));

    // Valid value in range.
    remote_options.remote_execution_priority = 10;
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::ExecutionAndCache,
    );
    assert!(st.is_ok());

    // Check not performed if the value is 0.
    remote_options.remote_execution_priority = 0;
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::ExecutionAndCache,
    );
    assert!(st.is_ok());

    // Ignored when no remote execution requested.
    remote_options.remote_execution_priority = 11;
    remote_options.remote_executor = Some(String::new());
    remote_options.remote_cache = Some("server:port".to_string());
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::Cache,
    );
    assert!(st.is_ok());
}

#[test]
fn test_check_client_server_compatibility_execution_caps_only() {
    let caps = ServerCapabilities {
        low_api_version: Some(ApiVersion::LOW.to_sem_ver()),
        high_api_version: Some(ApiVersion::HIGH.to_sem_ver()),
        execution_capabilities: Some(ExecutionCapabilities {
            digest_function: digest_function::Value::Sha256 as i32,
            exec_enabled: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut remote_options = RemoteOptions::default();
    remote_options.remote_executor = Some("server:port".to_string());
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::Execution,
    );
    assert!(st.is_ok());
}

#[test]
fn test_check_client_server_compatibility_execution_caps_digest_functions_list() {
    let caps = ServerCapabilities {
        low_api_version: Some(ApiVersion::LOW.to_sem_ver()),
        high_api_version: Some(ApiVersion::HIGH.to_sem_ver()),
        execution_capabilities: Some(ExecutionCapabilities {
            digest_functions: vec![
                digest_function::Value::Md5 as i32,
                digest_function::Value::Sha256 as i32,
            ],
            exec_enabled: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut remote_options = RemoteOptions::default();
    remote_options.remote_executor = Some("server:port".to_string());
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::Execution,
    );
    assert!(st.is_ok());
}

#[test]
fn test_check_client_server_compatibility_cache_caps_only() {
    let caps = ServerCapabilities {
        low_api_version: Some(ApiVersion::LOW.to_sem_ver()),
        high_api_version: Some(ApiVersion::HIGH.to_sem_ver()),
        cache_capabilities: Some(CacheCapabilities {
            digest_functions: vec![digest_function::Value::Sha256 as i32],
            action_cache_update_capabilities: Some(ActionCacheUpdateCapabilities {
                update_enabled: true,
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut remote_options = RemoteOptions::default();
    remote_options.remote_cache = Some("server:port".to_string());
    let st = check_client_server_compatibility(
        &caps,
        &remote_options,
        digest_function::Value::Sha256,
        ServerCapabilitiesRequirement::Cache,
    );
    assert!(st.is_ok());
}
