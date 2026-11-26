//! Credentials fetched from a credential helper, with a fallback.

use std::sync::Arc;

use super::{
    CredentialCache, CredentialHelperEnvironment, CredentialHelperProvider, GetCredentialsResponse,
};
use crate::authandtls::{Credentials, Headers};
use crate::errors::IoException;
use crate::uri::JavaUri;

/// Implementation of [`Credentials`] which fetches credentials by invoking a credential helper
/// as subprocess, falling back to another [`Credentials`] if no suitable helper exists.
///
/// PORT: `F` is the type of the fallback credentials: the downloader's [`Credentials`], or a
/// subtrait of it (the remote module's credentials also `refresh()`; `blaze_remote` implements
/// `refresh` for `CredentialHelperCredentials<dyn its Credentials>` with
/// [`Self::fallback_credentials`] and [`Self::invalidate_cache`]).
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperCredentials.java:33 CredentialHelperCredentials
pub struct CredentialHelperCredentials<F: ?Sized + Credentials = dyn Credentials> {
    credential_helper_provider: CredentialHelperProvider,
    credential_helper_environment: CredentialHelperEnvironment,
    credential_cache: Arc<CredentialCache>,
    fallback_credentials: Option<Arc<F>>,
}

impl<F: ?Sized + Credentials> CredentialHelperCredentials<F> {
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperCredentials.java:39 CredentialHelperCredentials.<init>
    pub fn new(
        credential_helper_provider: CredentialHelperProvider,
        credential_helper_environment: CredentialHelperEnvironment,
        credential_cache: Arc<CredentialCache>,
        fallback_credentials: Option<Arc<F>>,
    ) -> CredentialHelperCredentials<F> {
        CredentialHelperCredentials {
            credential_helper_provider,
            credential_helper_environment,
            credential_cache,
            fallback_credentials,
        }
    }

    /// The fallback credentials (Java's `fallbackCredentials` field).
    pub fn fallback_credentials(&self) -> Option<&Arc<F>> {
        self.fallback_credentials.as_ref()
    }

    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperCredentials.java:80 CredentialHelperCredentials.getCredentialsFromHelper
    fn get_credentials_from_helper(
        &self,
        uri: &JavaUri,
    ) -> Result<Option<GetCredentialsResponse>, IoException> {
        let Some(credential_helper) = self.credential_helper_provider.find_credential_helper(uri)
        else {
            return Ok(None);
        };

        let response =
            credential_helper.get_credentials(&self.credential_helper_environment, uri)?;
        Ok(Some(response))
    }

    /// `credentialCache.invalidateAll()`, the second half of `refresh()`.
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperCredentials.java:114 CredentialHelperCredentials.refresh
    pub fn invalidate_cache(&self) {
        self.credential_cache.invalidate_all();
    }

    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperCredentials.java:99 CredentialHelperCredentials.hasRequestMetadata
    pub fn has_request_metadata(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperCredentials.java:104 CredentialHelperCredentials.hasRequestMetadataOnly
    pub fn has_request_metadata_only(&self) -> bool {
        false
    }
}

impl<F: ?Sized + Credentials> Credentials for CredentialHelperCredentials<F> {
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperCredentials.java:60 CredentialHelperCredentials.getRequestMetadata
    fn get_request_metadata(&self, uri: &JavaUri) -> Result<Headers, IoException> {
        let response = self
            .credential_cache
            .get(uri, |uri| self.get_credentials_from_helper(uri))?;
        if let Some(response) = response {
            return Ok(response.headers().clone());
        }

        if let Some(fallback_credentials) = &self.fallback_credentials {
            return fallback_credentials.get_request_metadata(uri);
        }

        Ok(Headers::new())
    }
}
