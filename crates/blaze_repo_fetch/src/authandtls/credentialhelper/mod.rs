//! Port of `com.google.devtools.build.lib.authandtls.credentialhelper`: the
//! [credential helper protocol](https://github.com/EngFlow/credential-helper-spec)
//! (`--credential_helper`, `--credential_helper_timeout`, `--credential_helper_cache_duration`).
//!
//! It lives in `blaze_repo_fetch` because both of its users reach this crate: the downloader
//! (`blaze_repo_rules::setup`, Java `BazelRepositoryModule`) and the gRPC remote cache and
//! executor (`blaze_remote::authandtls::google_auth_utils::new_credentials`, Java `RemoteModule`).
//!
//! [`gson`], [`java_time`] and [`credential_cache`] stand in for the third-party pieces the Java
//! classes use (Gson, `java.time`, Caffeine).

pub mod credential_cache;
pub mod credential_cache_expiry;
pub mod credential_helper;
pub mod credential_helper_credentials;
pub mod credential_helper_environment;
pub mod credential_helper_exception;
pub mod credential_helper_provider;
pub mod credential_module;
pub mod get_credentials_request;
pub mod get_credentials_response;
pub mod gson;
pub mod java_time;
pub mod unchecked_exception;
pub mod wall_ticker;

pub use credential_cache::{CacheEntry, CredentialCache, Ticker};
pub use credential_cache_expiry::CredentialCacheExpiry;
pub use credential_helper::CredentialHelper;
pub use credential_helper_credentials::CredentialHelperCredentials;
pub use credential_helper_environment::{
    CredentialHelperEnvironment, CredentialHelperEnvironmentBuilder,
};
pub use credential_helper_exception::CredentialHelperException;
pub use credential_helper_provider::{CredentialHelperProvider, CredentialHelperProviderBuilder};
pub use credential_module::CredentialModule;
pub use get_credentials_request::{GetCredentialsRequest, GetCredentialsRequestBuilder};
pub use get_credentials_response::{GetCredentialsResponse, GetCredentialsResponseBuilder};
pub use gson::JsonSyntaxException;
pub use java_time::Instant;
pub use unchecked_exception::{UncheckedException, catch_unchecked};
pub use wall_ticker::WallTicker;
