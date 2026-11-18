//! Port of `ModuleKey`.

use std::cmp::Ordering;
use std::fmt;

use blaze_cmdline::RepositoryName;
use blaze_cmdline::repository_name::{BAZEL_TOOLS, MAIN};
use blaze_util::check_state;

use crate::version::{Version, VersionParseException, java_string_compare};

/// A module name, version pair that identifies a node in the external dependency graph.
///
/// `name` must be empty for the root module; `version` must be empty iff the module has a
/// `NonRegistryOverride`. Port of Java `ModuleKey`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleKey.java:35 ModuleKey
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ModuleKey {
    name: String,
    version: Version,
}

impl ModuleKey {
    pub fn new(name: impl Into<String>, version: Version) -> ModuleKey {
        ModuleKey {
            name: name.into(),
            version,
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleKey.java:61 ModuleKey.ROOT
    pub fn root() -> ModuleKey {
        ModuleKey {
            name: String::new(),
            version: Version::empty(),
        }
    }

    pub fn is_root(&self) -> bool {
        self.name.is_empty() && self.version.is_empty()
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> &Version {
        &self.version
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleKey.java:63 ModuleKey.LEXICOGRAPHIC_COMPARATOR
    pub fn lexicographic_cmp(a: &ModuleKey, b: &ModuleKey) -> Ordering {
        java_string_compare(&a.name, &b.name).then_with(|| a.version.cmp(&b.version))
    }

    /// Returns a string such as "root module" or "module 'foo@1.2.3'" for display purposes.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleKey.java:75 ModuleKey.toDisplayString
    pub fn to_display_string(&self) -> String {
        if self.is_root() {
            return "root module".to_string();
        }
        format!("module '{self}'")
    }

    /// Returns the canonical name of the repo backing this module, including its version. This
    /// name is always guaranteed to be unique. Must not be called if the module has a
    /// `NonRegistryOverride`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleKey.java:89 ModuleKey.getCanonicalRepoNameWithVersion
    pub fn get_canonical_repo_name_with_version(&self) -> RepositoryName {
        self.get_canonical_repo_name(true)
    }

    /// Returns the canonical name of the repo backing this module, excluding its version. This
    /// name is only guaranteed to be unique when there is a single version of the module in the
    /// entire dep graph.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleKey.java:98 ModuleKey.getCanonicalRepoNameWithoutVersion
    pub fn get_canonical_repo_name_without_version(&self) -> RepositoryName {
        self.get_canonical_repo_name(false)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleKey.java:102 ModuleKey.getCanonicalRepoName
    fn get_canonical_repo_name(&self, include_version: bool) -> RepositoryName {
        // WELL_KNOWN_MODULES. Keep in sync with src/tools/bzlmod/utils.bzl.
        // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleKey.java:47 ModuleKey.WELL_KNOWN_MODULES
        match self.name.as_str() {
            "bazel_tools" => return BAZEL_TOOLS.clone(),
            // Ensures that references to "@platforms" in WORKSPACE files resolve to the
            // repository of the "platforms" module.
            "platforms" => return RepositoryName::create_unvalidated("platforms"),
            _ => {}
        }
        if self.is_root() {
            return MAIN.clone();
        }
        let suffix = if include_version {
            // getVersion().isEmpty() is true only for modules with non-registry overrides, which
            // enforce that there is a single version of the module in the dep graph.
            check_state!(!self.version.is_empty());
            self.version.to_string()
        } else {
            // This results in canonical repository names such as `rules_foo+` for the module
            // `rules_foo`.
            String::new()
        };
        RepositoryName::create_unvalidated(&format!("{}+{}", self.name, suffix))
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleKey.java:130 ModuleKey.fromString
    pub fn from_string(s: &str) -> Result<ModuleKey, VersionParseException> {
        if s == "<root>" {
            return Ok(ModuleKey::root());
        }
        let parts: Vec<&str> = s.split('@').collect();
        // PORT: Java's parts.get(1) throws IndexOutOfBoundsException when there is no '@'.
        let Some(second) = parts.get(1) else {
            panic!(
                "IndexOutOfBoundsException: Index 1 out of bounds for length {}",
                parts.len()
            );
        };
        if *second == "_" {
            return Ok(ModuleKey::new(parts[0], Version::empty()));
        }
        let version = Version::parse(second)?;
        Ok(ModuleKey::new(parts[0], version))
    }
}

impl fmt::Display for ModuleKey {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleKey.java:66 ModuleKey.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_root() {
            return f.write_str("<root>");
        }
        if self.version.is_empty() {
            write!(f, "{}@_", self.name)
        } else {
            write!(f, "{}@{}", self.name, self.version)
        }
    }
}
