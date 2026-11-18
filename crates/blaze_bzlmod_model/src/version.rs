//! Port of `Version`.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::LazyLock;

use regex::Regex;

/// An exception encountered while trying to [parse](Version::parse) a version.
// Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:201 Version.ParseException
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionParseException {
    message: String,
    cause: Option<Box<VersionParseException>>,
}

impl VersionParseException {
    pub fn new(message: impl Into<String>) -> Self {
        VersionParseException {
            message: message.into(),
            cause: None,
        }
    }

    pub fn with_cause(message: impl Into<String>, cause: VersionParseException) -> Self {
        VersionParseException {
            message: message.into(),
            cause: Some(Box::new(cause)),
        }
    }

    /// Java `getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }

    /// Java `getCause()` (only set for wrapped identifier errors).
    pub fn get_cause(&self) -> Option<&VersionParseException> {
        self.cause.as_deref()
    }
}

impl fmt::Display for VersionParseException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for VersionParseException {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause
            .as_deref()
            .map(|c| c as &(dyn std::error::Error + 'static))
    }
}

/// Represents an "identifier", a dot-separated segment in the version string. An identifier is
/// compared differently based on whether it's digits-only or not.
// Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:84 Version.Identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VersionIdentifier {
    is_digits_only: bool,
    as_number: u64,
    as_string: String,
}

impl VersionIdentifier {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:86 Identifier.from
    fn from(string: &str) -> Result<VersionIdentifier, VersionParseException> {
        if string.is_empty() {
            return Err(VersionParseException::new("identifier is empty"));
        }
        // PORT: Character::isDigit accepts non-ASCII digits, but PATTERN only admits ASCII.
        if string.chars().all(|c| c.is_ascii_digit()) {
            match string.parse::<u64>() {
                Ok(n) => Ok(VersionIdentifier {
                    is_digits_only: true,
                    as_number: n,
                    as_string: string.to_string(),
                }),
                Err(_) => Err(VersionParseException::new(format!(
                    "numeric version segment is too large: {string}"
                ))),
            }
        } else {
            Ok(VersionIdentifier {
                is_digits_only: false,
                as_number: 0,
                as_string: string.to_string(),
            })
        }
    }

    pub fn is_digits_only(&self) -> bool {
        self.is_digits_only
    }

    pub fn as_number(&self) -> u64 {
        self.as_number
    }

    pub fn as_string(&self) -> &str {
        &self.as_string
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:101 Identifier.COMPARATOR
    fn compare(&self, o: &VersionIdentifier) -> Ordering {
        // comparing(isDigitsOnly, trueFirst())
        o.is_digits_only
            .cmp(&self.is_digits_only)
            .then_with(|| self.as_number.cmp(&o.as_number))
            .then_with(|| java_string_compare(&self.as_string, &o.as_string))
    }
}

impl Ord for VersionIdentifier {
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare(other)
    }
}

impl PartialOrd for VersionIdentifier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Java `String.compareTo` (UTF-16 code unit order).
pub(crate) fn java_string_compare(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Represents a version in the Bazel module system. The version format we support is
/// `RELEASE[-PRERELEASE][+BUILD]`, where `RELEASE`, `PRERELEASE`, and `BUILD` are each a sequence
/// of "identifiers" (defined as a non-empty sequence of ASCII alphanumerical characters and
/// hyphens) separated by dots. The `RELEASE` part may not contain hyphens.
///
/// Versions with a "build" part are generally accepted as input, but they're treated as if the
/// "build" part is completely absent. The special "empty string" version can also be used, and
/// compares higher than everything else. It signifies that there is a `NonRegistryOverride` for a
/// module. Port of Java `Version`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:50 Version
#[derive(Debug, Clone)]
pub struct Version {
    release: Vec<VersionIdentifier>,
    prerelease: Vec<VersionIdentifier>,
    normalized: String,
}

// We don't care about the "build" part at all so don't capture it.
// Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:55 Version.PATTERN
static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?P<release>[a-zA-Z0-9.]+)(?:-(?P<prerelease>[a-zA-Z0-9.-]+))?(?:\+[a-zA-Z0-9.-]+)?$",
    )
    .unwrap()
});

impl Version {
    /// Represents the special "empty string" version, which compares higher than everything else
    /// and signifies that there is a `NonRegistryOverride` for the module.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:65 Version.EMPTY
    pub fn empty() -> Version {
        Version {
            release: Vec::new(),
            prerelease: Vec::new(),
            normalized: String::new(),
        }
    }

    pub fn release(&self) -> &[VersionIdentifier] {
        &self.release
    }

    pub fn prerelease(&self) -> &[VersionIdentifier] {
        &self.prerelease
    }

    /// Returns the normalized version string (that is, with any "build" part stripped).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:112 Version.getNormalized
    pub fn get_normalized(&self) -> &str {
        &self.normalized
    }

    /// Whether this is just the "empty string" version, which signifies a non-registry override
    /// for the module.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:120 Version.isEmpty
    pub fn is_empty(&self) -> bool {
        self.normalized.is_empty()
    }

    /// Whether this is a prerelease version.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:129 Version.isPrerelease
    pub fn is_prerelease(&self) -> bool {
        !self.prerelease.is_empty()
    }

    /// Parses a version string into a [`Version`] object.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:134 Version.parse
    pub fn parse(version: &str) -> Result<Version, VersionParseException> {
        if version.is_empty() {
            return Ok(Version::empty());
        }
        let Some(captures) = PATTERN.captures(version) else {
            return Err(VersionParseException::new(format!(
                "bad version (does not match regex): {version}"
            )));
        };
        let release = captures.name("release").unwrap().as_str();
        let prerelease = captures.name("prerelease").map(|m| m.as_str());

        let mut release_split = Vec::new();
        for ident in release.split('.') {
            match VersionIdentifier::from(ident) {
                Ok(i) => release_split.push(i),
                Err(e) => {
                    return Err(VersionParseException::with_cause(
                        format!("error parsing version: {version}"),
                        e,
                    ));
                }
            }
        }

        let mut prerelease_split = Vec::new();
        if let Some(prerelease) = prerelease.filter(|p| !p.is_empty()) {
            for ident in prerelease.split('.') {
                match VersionIdentifier::from(ident) {
                    Ok(i) => prerelease_split.push(i),
                    Err(e) => {
                        return Err(VersionParseException::with_cause(
                            format!("error parsing version: {version}"),
                            e,
                        ));
                    }
                }
            }
        }

        let normalized = match prerelease.filter(|p| !p.is_empty()) {
            None => release.to_string(),
            Some(p) => format!("{release}-{p}"),
        };
        Ok(Version {
            release: release_split,
            prerelease: prerelease_split,
            normalized,
        })
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:168 Version.COMPARATOR
    fn compare(&self, o: &Version) -> Ordering {
        // comparing(isEmpty, falseFirst())
        self.is_empty()
            .cmp(&o.is_empty())
            .then_with(|| self.release.cmp(&o.release))
            // isPrerelease, trueFirst()
            .then_with(|| o.is_prerelease().cmp(&self.is_prerelease()))
            .then_with(|| self.prerelease.cmp(&o.prerelease))
    }
}

impl Default for Version {
    fn default() -> Self {
        Version::empty()
    }
}

impl Ord for Version {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:175 Version.compareTo
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare(other)
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Version {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:185 Version.equals
    fn eq(&self, other: &Self) -> bool {
        self.normalized == other.normalized
    }
}

impl Eq for Version {}

impl Hash for Version {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:190 Version.hashCode
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.normalized.hash(state);
    }
}

impl fmt::Display for Version {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Version.java:180 Version.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.normalized)
    }
}
