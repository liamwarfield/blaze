//! Port of the gRPC side of `com.google.devtools.build.lib.authandtls`: `CallCredentialsProvider`,
//! `GoogleAuthCallCredentialsProvider`, `GoogleAuthUtils` (channel construction, TLS, credentials)
//! and the channel-level header interceptors of `lib/remote/util/TracingMetadataUtils`.
//!
//! The Starlark-independent part of the package (`.netrc` parsing, `NetrcCredentials`,
//! `StaticCredentials`, `BasicHttpAuthenticationEncoder`) is ported in
//! `blaze_repo_fetch::authandtls`; this module builds on it.

pub mod call_credentials_provider;
pub mod google_auth_utils;
pub mod headers;
pub mod jdk_server_cert_verifier;

use std::io;

use blaze_repo_fetch::authandtls::{NetrcCredentials, StaticCredentials};

pub use blaze_repo_fetch::authandtls::credentialhelper::CredentialHelperEnvironment;
pub use call_credentials_provider::{
    CallCredentialsProvider, GoogleAuthCallCredentialsProvider, NO_CREDENTIALS, NoCredentials,
};
pub use google_auth_utils::{GoogleAuthLibraryCallCredentials, ManagedChannel};
pub use headers::{
    new_cache_headers_interceptor, new_downloader_headers_interceptor, new_exec_headers_interceptor,
};

/// `com.google.auth.Credentials` as the gRPC side uses it.
///
/// PORT: `blaze_repo_fetch::authandtls::Credentials` ports `getRequestMetadata(URI)` only (all
/// the downloader calls); the remote package also calls `refresh()`
/// (`GoogleAuthCallCredentialsProvider.refresh`), which is added by this trait.
// Java: com/google/auth/Credentials.java Credentials
pub trait Credentials: blaze_repo_fetch::authandtls::Credentials {
    /// `Credentials.refresh()`: refresh the authorization data, discarding any cached state.
    fn refresh(&self) -> io::Result<()>;
}

impl Credentials for NetrcCredentials {
    // Java: com/google/devtools/build/lib/authandtls/NetrcCredentials.java:77 NetrcCredentials.refresh
    fn refresh(&self) -> io::Result<()> {
        Ok(())
    }
}

impl Credentials for StaticCredentials {
    // Java: com/google/devtools/build/lib/authandtls/StaticCredentials.java:58 StaticCredentials.refresh
    fn refresh(&self) -> io::Result<()> {
        // Can't refresh static credentials.
        Ok(())
    }
}
