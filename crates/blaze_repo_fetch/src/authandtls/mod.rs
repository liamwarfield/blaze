//! Port of the parts of `com.google.devtools.build.lib.authandtls` the downloader uses.

pub mod netrc;
pub mod netrc_parser;

pub use netrc::{Netrc, NetrcCredential, NetrcCredentials};

use std::collections::HashMap;

use indexmap::IndexMap;

use crate::errors::IoException;
use crate::uri::JavaUri;

/// Request headers: name to values (Java `Map<String, List<String>>`, insertion ordered).
pub type Headers = IndexMap<String, Vec<String>>;

/// Port of `com.google.auth.Credentials` (the subset Bazel's downloader calls).
pub trait Credentials: Send + Sync {
    /// `Credentials.getRequestMetadata(URI)`.
    fn get_request_metadata(&self, uri: &JavaUri) -> Result<Headers, IoException>;
}

/// Implementation of [`Credentials`] which provides a static set of credentials.
// Java: com/google/devtools/build/lib/authandtls/StaticCredentials.java:27 StaticCredentials
#[derive(Clone, Debug, Default)]
pub struct StaticCredentials {
    credentials: HashMap<JavaUri, Headers>,
}

impl StaticCredentials {
    pub fn new(credentials: HashMap<JavaUri, Headers>) -> Self {
        StaticCredentials { credentials }
    }

    /// `StaticCredentials.EMPTY`.
    pub fn empty() -> Self {
        Self::default()
    }
}

impl Credentials for StaticCredentials {
    // Java: com/google/devtools/build/lib/authandtls/StaticCredentials.java:42 StaticCredentials.getRequestMetadata
    fn get_request_metadata(&self, uri: &JavaUri) -> Result<Headers, IoException> {
        Ok(self.credentials.get(uri).cloned().unwrap_or_default())
    }
}

/// Encodes a user name and password for Basic HTTP authentication (RFC 7617).
// Java: com/google/devtools/build/lib/authandtls/BasicHttpAuthenticationEncoder.java:37 BasicHttpAuthenticationEncoder.encode
pub fn basic_http_authentication_encode(username: &str, password: &str) -> String {
    use base64::Engine as _;
    // PORT: StringEncoding identity (internal strings are UTF-8 bytes).
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", username, password))
    )
}
