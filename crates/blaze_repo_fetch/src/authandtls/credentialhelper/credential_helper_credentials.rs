//! Port of `CredentialHelperCredentials`.

use std::sync::Arc;

use super::credential_cache::CredentialCache;
use super::credential_helper_environment::CredentialHelperEnvironment;
use super::credential_helper_provider::CredentialHelperProvider;
use super::get_credentials_response::GetCredentialsResponse;
use crate::authandtls::{Credentials, Headers};
use crate::errors::IoException;
use crate::uri::JavaUri;

/// Implementation of [`Credentials`] which fetches credentials by invoking a credential helper
/// as subprocess, falling back to another [`Credentials`] if no suitable helper exists.
///
/// PORT: `C` is the type of the fallback credentials: `dyn Credentials` for the downloader;
/// `blaze_remote` uses its own credentials trait (which adds `refresh()`) and implements it with
/// [`CredentialHelperCredentials::refresh`]. `getAuthenticationType`, `hasRequestMetadata` and
/// `hasRequestMetadataOnly` have no caller in the port and are not ported.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperCredentials.java:33 CredentialHelperCredentials
pub struct CredentialHelperCredentials<C: ?Sized + Credentials = dyn Credentials> {
    credential_helper_provider: Arc<CredentialHelperProvider>,
    credential_helper_environment: CredentialHelperEnvironment,
    credential_cache: Arc<CredentialCache>,
    fallback_credentials: Option<Arc<C>>,
}

impl<C: ?Sized + Credentials> CredentialHelperCredentials<C> {
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperCredentials.java:39 CredentialHelperCredentials.<init>
    pub fn new(
        credential_helper_provider: Arc<CredentialHelperProvider>,
        credential_helper_environment: CredentialHelperEnvironment,
        credential_cache: Arc<CredentialCache>,
        fallback_credentials: Option<Arc<C>>,
    ) -> CredentialHelperCredentials<C> {
        CredentialHelperCredentials {
            credential_helper_provider,
            credential_helper_environment,
            credential_cache,
            fallback_credentials,
        }
    }

    /// The fallback credentials.
    pub fn fallback_credentials(&self) -> Option<&Arc<C>> {
        self.fallback_credentials.as_ref()
    }

    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperCredentials.java:78 CredentialHelperCredentials.getCredentialsFromHelper
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

    /// `refresh()`: refreshes the fallback credentials with `refresh_fallback`, then drops every
    /// cached credential.
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperCredentials.java:114 CredentialHelperCredentials.refresh
    pub fn refresh<E>(&self, refresh_fallback: impl FnOnce(&C) -> Result<(), E>) -> Result<(), E> {
        if let Some(fallback_credentials) = &self.fallback_credentials {
            refresh_fallback(fallback_credentials)?;
        }

        self.credential_cache.invalidate_all();
        Ok(())
    }
}

impl<C: ?Sized + Credentials> Credentials for CredentialHelperCredentials<C> {
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperCredentials.java:60 CredentialHelperCredentials.getRequestMetadata
    fn get_request_metadata(&self, uri: &JavaUri) -> Result<Headers, IoException> {
        // The UncheckedIOException Java tunnels through the cache is the `Err` here.
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
