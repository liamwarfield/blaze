#![cfg(feature = "wip")]
//! Port of `src/test/java/com/google/devtools/build/lib/remote/ClientApiVersionTest.java`.
//!
//! The Java test is `Parameterized`; the Rust test runs the same cases in a loop.

use std::cmp::Ordering;

use blaze_remote::api_version::ApiVersion;
use blaze_remote::client_api_version::{ClientApiVersion, ServerSupportedStatus};
use blaze_remote::reapi::{SemVer, ServerCapabilities};

/// `SemVer.newBuilder().setMajor(major).setMinor(minor).build()`.
fn sem_ver(major: i32, minor: i32) -> SemVer {
    SemVer {
        major,
        minor,
        ..Default::default()
    }
}

/// `new ApiVersion(SemVer.newBuilder().setMajor(major).setMinor(minor).build())`.
fn api_version(major: i32, minor: i32) -> ApiVersion {
    ApiVersion::from_sem_ver(&sem_ver(major, minor))
}

struct TestCase {
    name: &'static str,
    client_api_version: ClientApiVersion,
    server_capabilities: ServerCapabilities,
    expected_highest_supported_version: ServerSupportedStatus,
    expected_messages: Vec<&'static str>,
}

fn test_cases() -> Vec<TestCase> {
    vec![
        TestCase {
            name: "noSupportedVersion",
            client_api_version: ClientApiVersion::new(api_version(2, 0), api_version(2, 0)),
            server_capabilities: ServerCapabilities {
                low_api_version: Some(sem_ver(2, 1)),
                high_api_version: Some(sem_ver(2, 2)),
                ..Default::default()
            },
            expected_highest_supported_version: ServerSupportedStatus::unsupported(
                &api_version(2, 0),
                &api_version(2, 0),
                &api_version(2, 1),
                &api_version(2, 2),
            ),
            expected_messages: vec!["not supported", "2.0 to 2.0", "2.1 to 2.2"],
        },
        TestCase {
            name: "deprecated",
            client_api_version: ClientApiVersion::new(api_version(2, 0), api_version(2, 0)),
            server_capabilities: ServerCapabilities {
                deprecated_api_version: Some(sem_ver(2, 0)),
                low_api_version: Some(sem_ver(2, 1)),
                high_api_version: Some(sem_ver(2, 2)),
                ..Default::default()
            },
            expected_highest_supported_version: ServerSupportedStatus::deprecated(
                api_version(2, 0),
                &api_version(2, 1),
                &api_version(2, 2),
            ),
            expected_messages: vec!["2.0 is deprecated", "2.1 to 2.2"],
        },
        TestCase {
            name: "clientHigh",
            client_api_version: ClientApiVersion::new(api_version(2, 0), api_version(2, 3)),
            server_capabilities: ServerCapabilities {
                low_api_version: Some(sem_ver(2, 0)),
                high_api_version: Some(sem_ver(2, 4)),
                ..Default::default()
            },
            expected_highest_supported_version: ServerSupportedStatus::supported(api_version(2, 3)),
            expected_messages: vec![],
        },
        TestCase {
            name: "serverHigh",
            client_api_version: ClientApiVersion::new(api_version(2, 0), api_version(2, 3)),
            server_capabilities: ServerCapabilities {
                low_api_version: Some(sem_ver(2, 0)),
                high_api_version: Some(sem_ver(2, 1)),
                ..Default::default()
            },
            expected_highest_supported_version: ServerSupportedStatus::supported(api_version(2, 1)),
            expected_messages: vec![],
        },
    ]
}

#[test]
fn test_client_api_version() {
    for case in test_cases() {
        let name = case.name;
        let server_supported_status = case
            .client_api_version
            .check_server_supported_versions(&case.server_capabilities);
        let expected_highest_supported_version = &case.expected_highest_supported_version;
        assert_eq!(
            server_supported_status.is_supported(),
            expected_highest_supported_version.is_supported(),
            "{name}"
        );
        assert_eq!(
            server_supported_status.is_unsupported(),
            expected_highest_supported_version.is_unsupported(),
            "{name}"
        );
        assert_eq!(
            server_supported_status.is_deprecated(),
            expected_highest_supported_version.is_deprecated(),
            "{name}"
        );
        assert_eq!(
            server_supported_status.get_message(),
            expected_highest_supported_version.get_message(),
            "{name}"
        );

        for expected_message in &case.expected_messages {
            assert!(
                server_supported_status
                    .get_message()
                    .contains(expected_message),
                "{name}: {:?} does not contain {expected_message:?}",
                server_supported_status.get_message()
            );
        }
        if case.expected_messages.is_empty() {
            assert!(server_supported_status.get_message().is_empty(), "{name}");
        }

        let expected_high = expected_highest_supported_version.get_highest_supported_version();
        if let Some(expected_high) = expected_high {
            let high = server_supported_status.get_highest_supported_version();

            let high = high.unwrap_or_else(|| panic!("{name}: no highest supported version"));
            assert_eq!(high.compare_to(expected_high), Ordering::Equal, "{name}");
        }
    }
}
