//! The part of `lib/authandtls/GoogleAuthUtils.java` that both the downloader and the gRPC side
//! use: `newCredentialHelperProvider`.
//!
//! PORT: the rest of the class (channels, Google and `.netrc` credentials, `newCredentials`) is
//! in `blaze_remote::authandtls::google_auth_utils`, which re-exports this function.

use blaze_optdefs::misc::auth_and_tls_options::CredentialHelperOption;

use super::credentialhelper::{CredentialHelperEnvironment, CredentialHelperProvider};
use crate::errors::IoException;
use crate::runtime::command_line_path_factory::CommandLinePathFactory;

/// `GoogleAuthUtils.newCredentialHelperProvider(environment, pathFactory, helpers)`: the
/// provider of the `--credential_helper` values, each path resolved with `path_factory` against
/// the client environment.
// Java: com/google/devtools/build/lib/authandtls/GoogleAuthUtils.java:416 GoogleAuthUtils.newCredentialHelperProvider
pub fn new_credential_helper_provider(
    environment: &CredentialHelperEnvironment,
    path_factory: &CommandLinePathFactory,
    helpers: &[CredentialHelperOption],
) -> Result<CredentialHelperProvider, IoException> {
    let mut builder = CredentialHelperProvider::builder();
    for helper in helpers {
        let path = path_factory
            .create(environment.client_environment(), &helper.path)
            .map_err(IoException::from)?;
        match &helper.scope {
            Some(scope) => builder.add_for_pattern(scope, path),
            None => builder.add(path),
        };
    }
    Ok(builder.build())
}
