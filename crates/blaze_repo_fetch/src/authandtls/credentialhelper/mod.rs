//! Port of `com.google.devtools.build.lib.authandtls.credentialhelper`: the credential helper
//! protocol (<https://github.com/bazelbuild/proposals/blob/main/designs/2022-06-07-bazel-credential-helpers.md>)
//! and the credential cache that `CredentialModule` holds for the server.
//!
//! PORT: the package lives in this crate (not with `GoogleAuthUtils` in `blaze_remote`) because
//! its three users are here or above it: the downloader (`BazelRepositoryModule`'s credential
//! factory, which Bzlmod registries share), the remote module and the build event service.
//! `lib/runtime/CommandLinePathFactory`, which `GoogleAuthUtils.newCredentialHelperProvider`
//! takes, is ported next to it for the same reason ([`command_line_path_factory`]).

pub mod command_line_path_factory;
pub mod credential_cache;
pub mod credential_helper;
pub mod credential_helper_credentials;
pub mod credential_helper_environment;
pub mod credential_helper_provider;
pub mod get_credentials_request;
pub mod get_credentials_response;
mod gson;

pub use command_line_path_factory::CommandLinePathFactory;
pub use credential_cache::{
    CacheEntry, CredentialCache, CredentialCacheExpiry, CredentialModule, WallTicker,
    credential_module,
};
pub use credential_helper::CredentialHelper;
pub use credential_helper_credentials::CredentialHelperCredentials;
pub use credential_helper_environment::CredentialHelperEnvironment;
pub use credential_helper_provider::{CredentialHelperProvider, CredentialHelperProviderBuilder};
pub use get_credentials_request::GetCredentialsRequest;
pub use get_credentials_response::{GetCredentialsResponse, JavaInstant};
pub use gson::JsonSyntaxException;

use crate::errors::{IoException, IoExceptionKind};

/// `new CredentialHelperException(message)`: an `IOException` thrown while invoking a
/// credential helper.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperException.java:20 CredentialHelperException
pub fn credential_helper_exception(message: impl Into<String>) -> IoException {
    IoException::of(IoExceptionKind::CredentialHelperException, message)
}

/// Java `e instanceof CredentialHelperException`.
pub fn is_credential_helper_exception(e: &IoException) -> bool {
    e.kind == IoExceptionKind::CredentialHelperException
}
