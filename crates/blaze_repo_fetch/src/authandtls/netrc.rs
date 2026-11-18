//! Ports of `Netrc` and `NetrcCredentials`.

use std::collections::HashMap;
use std::fmt;

use indexmap::IndexMap;

use super::{Credentials, Headers, basic_http_authentication_encode};
use crate::errors::IoException;
use crate::uri::JavaUri;

/// Container for login, password and account of a machine in .netrc.
// Java: com/google/devtools/build/lib/authandtls/Netrc.java:63 Netrc.Credential
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct NetrcCredential {
    pub machine: String,
    pub login: String,
    pub password: String,
    pub account: String,
}

impl NetrcCredential {
    /// `Credential.builder(machine)` with empty login, password and account.
    pub fn builder(machine: &str) -> NetrcCredential {
        NetrcCredential {
            machine: machine.to_string(),
            login: String::new(),
            password: String::new(),
            account: String::new(),
        }
    }

    pub fn set_login(mut self, v: &str) -> Self {
        self.login = v.to_string();
        self
    }

    pub fn set_password(mut self, v: &str) -> Self {
        self.password = v.to_string();
        self
    }

    pub fn set_account(mut self, v: &str) -> Self {
        self.account = v.to_string();
        self
    }
}

impl fmt::Debug for NetrcCredential {
    /// Does not leak the password.
    // Java: com/google/devtools/build/lib/authandtls/Netrc.java:76 Credential.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Credential{{machine={}, login={}, password=<password>, account={}}}",
            self.machine, self.login, self.account
        )
    }
}

/// Container for the content of a .netrc file.
// Java: com/google/devtools/build/lib/authandtls/Netrc.java:26 Netrc
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Netrc {
    pub default_credential: Option<NetrcCredential>,
    pub credentials: HashMap<String, NetrcCredential>,
}

impl Netrc {
    pub fn create(
        default_credential: Option<NetrcCredential>,
        credentials: HashMap<String, NetrcCredential>,
    ) -> Netrc {
        Netrc {
            default_credential,
            credentials,
        }
    }

    /// The credential for a machine, else the default one.
    // Java: com/google/devtools/build/lib/authandtls/Netrc.java:56 Netrc.getCredential
    pub fn get_credential(&self, machine: &str) -> Option<&NetrcCredential> {
        self.credentials
            .get(machine)
            .or(self.default_credential.as_ref())
    }
}

/// [`Credentials`] using the user name and password from a [`Netrc`].
// Java: com/google/devtools/build/lib/authandtls/NetrcCredentials.java:30 NetrcCredentials
#[derive(Clone, Debug)]
pub struct NetrcCredentials {
    netrc: Netrc,
}

impl NetrcCredentials {
    pub fn new(netrc: Netrc) -> Self {
        NetrcCredentials { netrc }
    }
}

impl Credentials for NetrcCredentials {
    // Java: com/google/devtools/build/lib/authandtls/NetrcCredentials.java:56 NetrcCredentials.getRequestMetadata
    fn get_request_metadata(&self, uri: &JavaUri) -> Result<Headers, IoException> {
        // PORT: Java passes a null host to Map.getOrDefault, which then yields the default.
        let credential = match uri.get_host() {
            Some(h) => self.netrc.get_credential(h),
            None => self.netrc.default_credential.as_ref(),
        };
        let mut m = IndexMap::new();
        if let Some(c) = credential {
            m.insert(
                "Authorization".to_string(),
                vec![basic_http_authentication_encode(&c.login, &c.password)],
            );
        }
        Ok(m)
    }
}
