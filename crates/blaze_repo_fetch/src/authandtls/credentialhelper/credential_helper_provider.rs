//! A provider for credential helpers.

use std::collections::HashMap;

use blaze_vfs::Path;

use super::CredentialHelper;
use crate::errors::IoException;
use crate::uri::JavaUri;

/// A provider for [`CredentialHelper`]s.
///
/// This class is used to find the right [`CredentialHelper`] for a URI, using the most specific
/// match.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperProvider.java:37 CredentialHelperProvider
#[derive(Clone, Debug, Default)]
pub struct CredentialHelperProvider {
    default_helper: Option<Path>,
    host_to_helper: HashMap<String, Path>,
    suffix_to_helper: HashMap<String, Path>,
}

impl CredentialHelperProvider {
    /// Returns the [`CredentialHelper`] to use for getting credentials for connection to the
    /// provided URI, or `None` if no [`CredentialHelper`] is configured for it.
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperProvider.java:64 CredentialHelperProvider.findCredentialHelper
    pub fn find_credential_helper(&self, uri: &JavaUri) -> Option<CredentialHelper> {
        let host = match uri.get_host() {
            Some(host) if !host.is_empty() => host,
            _ => {
                // Some URIs (e.g. unix://) legitimately have no host component.
                // Use the default helper if one is provided.
                return self.default_helper.clone().map(CredentialHelper::new);
            }
        };

        let credential_helper = self
            .find_host_credential_helper(host)
            .or_else(|| self.find_wildcard_credential_helper(host))
            .or_else(|| self.default_helper.clone());
        credential_helper.map(CredentialHelper::new)
    }

    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperProvider.java:81 CredentialHelperProvider.findHostCredentialHelper
    fn find_host_credential_helper(&self, host: &str) -> Option<Path> {
        self.host_to_helper.get(host).cloned()
    }

    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperProvider.java:87 CredentialHelperProvider.findWildcardCredentialHelper
    fn find_wildcard_credential_helper(&self, host: &str) -> Option<Path> {
        self.suffix_to_helper.get(host).cloned().or_else(|| {
            let subdomain = parent_domain(host)?;
            self.find_wildcard_credential_helper(subdomain)
        })
    }

    /// Returns a new builder for a [`CredentialHelperProvider`].
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperProvider.java:118 CredentialHelperProvider.builder
    pub fn builder() -> CredentialHelperProviderBuilder {
        CredentialHelperProviderBuilder::default()
    }
}

/// Returns the parent domain of the provided domain (e.g., `foo.example.com` for
/// `bar.foo.example.com`).
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperProvider.java:107 CredentialHelperProvider.parentDomain
pub fn parent_domain(domain: &str) -> Option<&str> {
    // We reached the last segment, end.
    let dot = domain.find('.')?;
    Some(&domain[dot + 1..])
}

/// Builder for [`CredentialHelperProvider`].
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperProvider.java:123 CredentialHelperProvider.Builder
#[derive(Debug, Default)]
pub struct CredentialHelperProviderBuilder {
    default_helper: Option<Path>,
    host_to_helper: HashMap<String, Path>,
    suffix_to_helper: HashMap<String, Path>,
}

impl CredentialHelperProviderBuilder {
    /// Adds a default credential helper to use for all URIs that don't specify a more specific
    /// credential helper.
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperProvider.java:133 CredentialHelperProvider.Builder.add
    pub fn add_default(&mut self, helper: Path) -> Result<&mut Self, IoException> {
        self.default_helper = Some(helper);
        Ok(self)
    }

    /// Adds a credential helper to use for all URIs matching the provided pattern.
    ///
    /// If `pattern` starts with a `*.` wildcard, it matches every subdomain in addition to the
    /// domain itself. For example `*.example.com` would match `example.com`, `foo.example.com`,
    /// `bar.example.com`, `baz.bar.example.com` and so on, but not anything that isn't a
    /// subdomain of `example.com`.
    ///
    /// More complex wildcard patterns are not supported.
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperProvider.java:153 CredentialHelperProvider.Builder.add
    pub fn add(&mut self, pattern: &str, helper: Path) -> Result<&mut Self, IoException> {
        // The pattern has already been normalized during options parsing.
        if let Some(suffix) = pattern.strip_prefix("*.") {
            self.suffix_to_helper.insert(suffix.to_string(), helper);
        } else {
            self.host_to_helper.insert(pattern.to_string(), helper);
        }

        Ok(self)
    }

    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperProvider.java:167 CredentialHelperProvider.Builder.build
    pub fn build(&self) -> CredentialHelperProvider {
        CredentialHelperProvider {
            default_helper: self.default_helper.clone(),
            host_to_helper: self.host_to_helper.clone(),
            suffix_to_helper: self.suffix_to_helper.clone(),
        }
    }
}
