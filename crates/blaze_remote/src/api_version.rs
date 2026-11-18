//! Port of `lib/remote/ApiVersion.java`.

use std::cmp::Ordering;
use std::fmt;

use crate::reapi::SemVer;

/// Represents a version of the Remote Execution API.
///
/// PORT: `Comparable<ApiVersion>` is `Ord`; equality is `compareTo(other) == 0` so that it agrees
/// with the order (the Java class does not override `equals`).
// Java: com/google/devtools/build/lib/remote/ApiVersion.java:20 ApiVersion
#[derive(Debug, Clone)]
pub struct ApiVersion {
    pub major: i32,
    pub minor: i32,
    pub patch: i32,
    pub prerelease: String,
}

impl ApiVersion {
    /// The version of the Remote Execution API that Bazel supports initially.
    // Java: com/google/devtools/build/lib/remote/ApiVersion.java:27 ApiVersion.twoPointZero
    pub const TWO_POINT_ZERO: ApiVersion = ApiVersion {
        major: 2,
        minor: 0,
        patch: 0,
        prerelease: String::new(),
    };
    /// The version of the Remote Execution API that starts supporting the
    /// Command.output_paths and ActionResult.output_symlinks fields.
    // Java: com/google/devtools/build/lib/remote/ApiVersion.java:31 ApiVersion.twoPointOne
    pub const TWO_POINT_ONE: ApiVersion = ApiVersion {
        major: 2,
        minor: 1,
        patch: 0,
        prerelease: String::new(),
    };
    /// The latest version of the Remote Execution API that Bazel is compatible with.
    // Java: com/google/devtools/build/lib/remote/ApiVersion.java:34 ApiVersion.twoPointEleven
    pub const TWO_POINT_ELEVEN: ApiVersion = ApiVersion {
        major: 2,
        minor: 11,
        patch: 0,
        prerelease: String::new(),
    };

    // The current lowest/highest versions (inclusive) of the Remote Execution API that Bazel
    // supports. These fields will need to be updated together with all version changes.
    // Java: com/google/devtools/build/lib/remote/ApiVersion.java:39 ApiVersion.low
    pub const LOW: ApiVersion = ApiVersion::TWO_POINT_ZERO;
    // Java: com/google/devtools/build/lib/remote/ApiVersion.java:40 ApiVersion.high
    pub const HIGH: ApiVersion = ApiVersion::TWO_POINT_ELEVEN;

    // Java: com/google/devtools/build/lib/remote/ApiVersion.java:42 ApiVersion.<init>
    pub fn new(major: i32, minor: i32, patch: i32, prerelease: &str) -> ApiVersion {
        ApiVersion {
            major,
            minor,
            patch,
            prerelease: prerelease.to_string(),
        }
    }

    /// `new ApiVersion(SemVer semver)`.
    // Java: com/google/devtools/build/lib/remote/ApiVersion.java:49 ApiVersion.<init>
    pub fn from_sem_ver(semver: &SemVer) -> ApiVersion {
        ApiVersion::new(semver.major, semver.minor, semver.patch, &semver.prerelease)
    }

    // Java: com/google/devtools/build/lib/remote/ApiVersion.java:69 ApiVersion.toSemVer
    pub fn to_sem_ver(&self) -> SemVer {
        SemVer {
            major: self.major,
            minor: self.minor,
            patch: self.patch,
            prerelease: self.prerelease.clone(),
        }
    }

    /// Compares the current API version to another API version.
    ///
    /// Returns `Equal` if the API versions are equal, `Less` if the current version is earlier
    /// than other, and `Greater` if the current version is later than other. It is assumed that
    /// all prerelease versions are earlier than all released versions.
    // Java: com/google/devtools/build/lib/remote/ApiVersion.java:87 ApiVersion.compareTo
    pub fn compare_to(&self, other: &ApiVersion) -> Ordering {
        if !self.prerelease.is_empty() {
            if other.prerelease.is_empty() {
                return Ordering::Less;
            }
            return java_string_compare(&self.prerelease, &other.prerelease);
        }
        if !other.prerelease.is_empty() {
            return Ordering::Greater;
        }
        if self.major != other.major {
            return self.major.cmp(&other.major);
        }
        if self.minor != other.minor {
            return self.minor.cmp(&other.minor);
        }
        self.patch.cmp(&other.patch)
    }
}

/// `String.compareTo`: lexicographic order of UTF-16 code units.
fn java_string_compare(a: &str, b: &str) -> Ordering {
    if a.is_ascii() && b.is_ascii() {
        return a.cmp(b);
    }
    a.encode_utf16().cmp(b.encode_utf16())
}

impl fmt::Display for ApiVersion {
    // Java: com/google/devtools/build/lib/remote/ApiVersion.java:54 ApiVersion.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.prerelease.is_empty() {
            return f.write_str(&self.prerelease);
        }
        let mut builder = String::new();
        builder.push_str(&self.major.to_string());
        builder.push('.');
        builder.push_str(&self.minor.to_string());
        if self.patch != 0 {
            builder.push('.');
            builder.push_str(&self.patch.to_string());
        }
        f.write_str(&builder)
    }
}

impl Ord for ApiVersion {
    fn cmp(&self, other: &ApiVersion) -> Ordering {
        self.compare_to(other)
    }
}

impl PartialOrd for ApiVersion {
    fn partial_cmp(&self, other: &ApiVersion) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for ApiVersion {
    fn eq(&self, other: &ApiVersion) -> bool {
        self.compare_to(other) == Ordering::Equal
    }
}

impl Eq for ApiVersion {}
