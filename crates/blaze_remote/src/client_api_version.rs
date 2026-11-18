//! Port of `lib/remote/ClientApiVersion.java`.

use std::cmp::Ordering;

use crate::api_version::ApiVersion;
use crate::reapi::ServerCapabilities;

/// Represents a range of the Remote Execution API that client supports.
// Java: com/google/devtools/build/lib/remote/ClientApiVersion.java:23 ClientApiVersion
#[derive(Debug, Clone)]
pub struct ClientApiVersion {
    low: ApiVersion,
    high: ApiVersion,
}

// Java: com/google/devtools/build/lib/remote/ClientApiVersion.java:48 ClientApiVersion.ServerSupportedStatus.State
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Supported,
    Unsupported,
    Deprecated,
}

/// `ClientApiVersion.ServerSupportedStatus`: whether, and up to which version, a server supports
/// the API versions of the client.
// Java: com/google/devtools/build/lib/remote/ClientApiVersion.java:47 ClientApiVersion.ServerSupportedStatus
#[derive(Debug, Clone)]
pub struct ServerSupportedStatus {
    message: String,
    state: State,
    highest_supported_version: Option<ApiVersion>,
}

impl ServerSupportedStatus {
    // Java: com/google/devtools/build/lib/remote/ClientApiVersion.java:58 ClientApiVersion.ServerSupportedStatus.<init>
    fn new(
        state: State,
        message: String,
        highest_supported_version: Option<ApiVersion>,
    ) -> ServerSupportedStatus {
        ServerSupportedStatus {
            state,
            message,
            highest_supported_version,
        }
    }

    // Java: com/google/devtools/build/lib/remote/ClientApiVersion.java:64 ClientApiVersion.ServerSupportedStatus.supported
    pub fn supported(highest_supported_version: ApiVersion) -> ServerSupportedStatus {
        ServerSupportedStatus::new(
            State::Supported,
            String::new(),
            Some(highest_supported_version),
        )
    }

    // Java: com/google/devtools/build/lib/remote/ClientApiVersion.java:68 ClientApiVersion.ServerSupportedStatus.unsupported
    pub fn unsupported(
        client_low: &ApiVersion,
        client_high: &ApiVersion,
        server_low: &ApiVersion,
        server_high: &ApiVersion,
    ) -> ServerSupportedStatus {
        ServerSupportedStatus::new(
            State::Unsupported,
            format!(
                "The client supported API versions, {client_low} to {client_high}, is not supported by the server, {server_low} to {server_high}. Please switch to a different server or upgrade Bazel."
            ),
            None,
        )
    }

    // Java: com/google/devtools/build/lib/remote/ClientApiVersion.java:79 ClientApiVersion.ServerSupportedStatus.deprecated
    pub fn deprecated(
        client_high: ApiVersion,
        server_low: &ApiVersion,
        server_high: &ApiVersion,
    ) -> ServerSupportedStatus {
        ServerSupportedStatus::new(
            State::Deprecated,
            format!(
                "The highest API version Bazel support {client_high} is deprecated by the server. Please upgrade to server's recommended version: {server_low} to {server_high}."
            ),
            Some(client_high),
        )
    }

    // Java: com/google/devtools/build/lib/remote/ClientApiVersion.java:90 ClientApiVersion.ServerSupportedStatus.getMessage
    pub fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/build/lib/remote/ClientApiVersion.java:94 ClientApiVersion.ServerSupportedStatus.getHighestSupportedVersion
    pub fn get_highest_supported_version(&self) -> Option<&ApiVersion> {
        self.highest_supported_version.as_ref()
    }

    // Java: com/google/devtools/build/lib/remote/ClientApiVersion.java:98 ClientApiVersion.ServerSupportedStatus.isSupported
    pub fn is_supported(&self) -> bool {
        self.state == State::Supported
    }

    // Java: com/google/devtools/build/lib/remote/ClientApiVersion.java:102 ClientApiVersion.ServerSupportedStatus.isDeprecated
    pub fn is_deprecated(&self) -> bool {
        self.state == State::Deprecated
    }

    // Java: com/google/devtools/build/lib/remote/ClientApiVersion.java:106 ClientApiVersion.ServerSupportedStatus.isUnsupported
    pub fn is_unsupported(&self) -> bool {
        self.state == State::Unsupported
    }
}

/// `Comparators.max(a, b)`: if the values compare as equal, the first is returned.
// Java: com/google/common/collect/Comparators.java Comparators.max
fn max<'a>(a: &'a ApiVersion, b: &'a ApiVersion) -> &'a ApiVersion {
    if a.compare_to(b) != Ordering::Less {
        a
    } else {
        b
    }
}

/// `Comparators.min(a, b)`: if the values compare as equal, the first is returned.
// Java: com/google/common/collect/Comparators.java Comparators.min
fn min<'a>(a: &'a ApiVersion, b: &'a ApiVersion) -> &'a ApiVersion {
    if a.compare_to(b) != Ordering::Greater {
        a
    } else {
        b
    }
}

impl ClientApiVersion {
    // Java: com/google/devtools/build/lib/remote/ClientApiVersion.java:27 ClientApiVersion.current
    pub const CURRENT: ClientApiVersion = ClientApiVersion {
        low: ApiVersion::LOW,
        high: ApiVersion::HIGH,
    };

    // Java: com/google/devtools/build/lib/remote/ClientApiVersion.java:30 ClientApiVersion.<init>
    pub fn new(low: ApiVersion, high: ApiVersion) -> ClientApiVersion {
        ClientApiVersion { low, high }
    }

    // Java: com/google/devtools/build/lib/remote/ClientApiVersion.java:35 ClientApiVersion.getLow
    pub fn get_low(&self) -> &ApiVersion {
        &self.low
    }

    // Java: com/google/devtools/build/lib/remote/ClientApiVersion.java:39 ClientApiVersion.getHigh
    pub fn get_high(&self) -> &ApiVersion {
        &self.high
    }

    // Java: com/google/devtools/build/lib/remote/ClientApiVersion.java:43 ClientApiVersion.isSupported
    pub fn is_supported(&self, version: &ApiVersion) -> bool {
        self.low.compare_to(version) != Ordering::Greater
            && self.high.compare_to(version) != Ordering::Less
    }

    // highestSupportedVersion compares the client's supported versions against the input low and
    // high versions and returns the highest supported version. If the client's supported versions
    // are not supported by the server, it returns null.
    // Java: com/google/devtools/build/lib/remote/ClientApiVersion.java:115 ClientApiVersion.highestSupportedVersion
    fn highest_supported_version(
        &self,
        server_low: &ApiVersion,
        server_high: &ApiVersion,
    ) -> Option<ApiVersion> {
        let higest_low = max(&self.low, server_low);
        let lowest_high = min(&self.high, server_high);

        if higest_low.compare_to(lowest_high) != Ordering::Greater {
            Some(lowest_high.clone())
        } else {
            None
        }
    }

    // Java: com/google/devtools/build/lib/remote/ClientApiVersion.java:122 ClientApiVersion.checkServerSupportedVersions
    pub fn check_server_supported_versions(
        &self,
        cap: &ServerCapabilities,
    ) -> ServerSupportedStatus {
        // PORT: an unset message field reads as the default instance (`getLowApiVersion()`).
        let server_low = ApiVersion::from_sem_ver(&cap.low_api_version.clone().unwrap_or_default());
        let server_high =
            ApiVersion::from_sem_ver(&cap.high_api_version.clone().unwrap_or_default());

        let highest = self.highest_supported_version(&server_low, &server_high);
        if let Some(highest) = highest {
            return ServerSupportedStatus::supported(highest);
        }

        let deprecated = cap
            .deprecated_api_version
            .as_ref()
            .map(ApiVersion::from_sem_ver);
        let Some(deprecated) = deprecated else {
            return ServerSupportedStatus::unsupported(
                &self.low,
                &self.high,
                &server_low,
                &server_high,
            );
        };

        let highest = self.highest_supported_version(&deprecated, &server_high);
        if let Some(highest) = highest {
            return ServerSupportedStatus::deprecated(highest, &server_low, &server_high);
        }

        ServerSupportedStatus::unsupported(&self.low, &self.high, &server_low, &server_high)
    }
}
