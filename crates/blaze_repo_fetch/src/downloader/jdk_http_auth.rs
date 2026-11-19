//! The JDK state and proxy authentication that Bazel's downloader runs on.
//!
//! PORT: Bazel downloads with `java.net.HttpURLConnection` (`sun.net.www.protocol.http`), and
//! `ProxyHelper` configures it through JVM-wide state: the default `Authenticator` and the system
//! property `jdk.http.auth.tunneling.disabledSchemes`. Neither the JDK's classes nor its source
//! are part of Bazel's tree (the oracle's embedded JDK, Zulu 25.0.2, ships no `src.zip`), so this
//! module reproduces what the oracle does, as recorded with `scripts/proxy` (recordings in
//! `tests/data/proxy`, one directory per scenario, described in its `README.md`):
//!
//! - credentials are never sent before the proxy asks: a `CONNECT` or a request through the proxy
//!   gets `Proxy-authorization` only in answer to a 407, or from the authentication cache after an
//!   earlier answer to the same proxy succeeded;
//! - the scheme is chosen among the 407's `Proxy-Authenticate` challenges in the order Negotiate,
//!   Kerberos, Digest, NTLM, Basic; there is no fall back to Basic when the chosen scheme cannot
//!   answer;
//! - `HttpURLConnection` reads `jdk.http.auth.tunneling.disabledSchemes` once, in its static
//!   initialiser, from the system property or else the JDK's `conf/net.properties` (`Basic`).
//!   `ProxyHelper` clears the property when it first sees proxy credentials; if an
//!   `HttpURLConnection` was created earlier in the server's life (a direct download), Basic stays
//!   disabled for tunnels and a `CONNECT` challenge for Basic is never answered;
//! - after a 407 the next request goes on a new connection, up to `http.maxRedirects` (20)
//!   requests per connection object;
//! - Basic encodes `user:password` (the `Authenticator`'s Unicode strings) as ISO-8859-1 (`?` for
//!   a char above U+00FF), or as UTF-8 when the challenge says `charset="UTF-8"`.
//!
//! Digest, NTLM and Negotiate/Kerberos, which the JDK can answer, are not ported: they fail with
//! `unported!` at the point where the JDK would compute the answer.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use blaze_util::string_encoding::latin1_encode_lossy;

/// `"Java/" + System.getProperty("java.version")`: the `User-Agent` the JDK writes on a
/// `CONNECT` (`http.agent` is not set by Bazel). The version is the oracle's embedded JDK, as
/// recorded on the wire.
pub const JDK_USER_AGENT: &str = "Java/25.0.2";

/// `http.maxRedirects` (default 20): the JDK's bound on requests per connection object while it
/// answers authentication challenges. Recorded: 20 requests, then the JDK gives up.
pub const MAX_REDIRECTS: usize = 20;

/// The name the JDK gives the header it sets itself (`AuthenticationInfo.getHeaderName` for a
/// proxy), as recorded.
pub const PROXY_AUTHORIZATION: &str = "Proxy-authorization";

const TUNNELING_DISABLED_SCHEMES: &str = "jdk.http.auth.tunneling.disabledSchemes";
const PROXYING_DISABLED_SCHEMES: &str = "jdk.http.auth.proxying.disabledSchemes";

/// The values of the JDK's `conf/net.properties` (the oracle's embedded JDK) for the properties
/// read here: `jdk.http.auth.tunneling.disabledSchemes=Basic`; the proxying one is commented out.
fn net_properties_default(key: &str) -> Option<&'static str> {
    match key {
        TUNNELING_DISABLED_SCHEMES => Some("Basic"),
        _ => None,
    }
}

/// `HttpURLConnection.schemesListToSet`: comma-separated, trimmed, lower-cased.
fn schemes_list_to_set(s: Option<&str>) -> HashSet<String> {
    match s {
        None => HashSet::new(),
        Some("") => HashSet::new(),
        Some(s) => s
            .split(',')
            .map(|x| x.trim().to_ascii_lowercase())
            .collect(),
    }
}

/// What `HttpURLConnection`'s static initialiser reads.
#[derive(Debug)]
struct HttpUrlConnectionStatics {
    disabled_tunneling_schemes: HashSet<String>,
    disabled_proxying_schemes: HashSet<String>,
}

/// `PasswordAuthentication` as Bazel's `Authenticator` returns it: Unicode strings
/// (`internalToUnicode` of ProxyHelper's internal strings).
#[derive(Clone, Debug, PartialEq, Eq)]
struct PasswordAuthentication {
    user_name: String,
    password: String,
}

/// The JDK's authentication schemes (`AuthScheme`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AuthScheme {
    Basic,
    Digest,
    Ntlm,
    Negotiate,
    Kerberos,
}

/// An answer to a proxy challenge (`AuthenticationInfo` for a proxy).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticationInfo {
    scheme: AuthScheme,
    host: String,
    port: i32,
    realm: String,
    header_value: String,
}

impl AuthenticationInfo {
    /// The header value (`Basic <base64>`).
    pub fn header_value(&self) -> &str {
        &self.header_value
    }
}

/// One `Proxy-Authenticate` challenge: the scheme name (lower case) and its parameters.
#[derive(Debug)]
struct Challenge {
    scheme: String,
    params: Vec<(String, String)>,
}

impl Challenge {
    /// `HeaderParser.findValue`: the first parameter of that name, ignoring case.
    fn find_value(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

/// Parses one `Proxy-Authenticate` value: the scheme token, then `key=value` or
/// `key="quoted value"` parameters separated by commas.
///
/// PORT: one challenge per header line, as every recorded proxy sends them. A line that holds
/// several challenges is read as one challenge with the first scheme.
fn parse_challenge(raw: &str) -> Challenge {
    let raw = raw.trim();
    let (scheme, rest) = match raw.find([' ', '\t']) {
        Some(i) => (&raw[..i], &raw[i + 1..]),
        None => (raw, ""),
    };
    let mut params = Vec::new();
    let mut chars = rest.chars().peekable();
    loop {
        while chars
            .peek()
            .is_some_and(|c| *c == ',' || c.is_ascii_whitespace())
        {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let mut key = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' || c == ',' {
                break;
            }
            key.push(c);
            chars.next();
        }
        let mut value = String::new();
        if chars.peek() == Some(&'=') {
            chars.next();
            if chars.peek() == Some(&'"') {
                chars.next();
                for c in chars.by_ref() {
                    if c == '"' {
                        break;
                    }
                    value.push(c);
                }
            } else {
                while let Some(&c) = chars.peek() {
                    if c == ',' {
                        break;
                    }
                    value.push(c);
                    chars.next();
                }
            }
        }
        params.push((key.trim().to_string(), value.trim().to_string()));
    }
    Challenge {
        scheme: scheme.to_ascii_lowercase(),
        params,
    }
}

/// `AuthenticationHeader`: picks the challenge to answer among the response's
/// `Proxy-Authenticate` values, skipping disabled schemes. Order: Negotiate, Kerberos, Digest,
/// NTLM, Basic (recorded: NTLM is chosen over Basic, an unusable Digest over Basic, and nothing
/// falls back to Basic).
fn select_challenge(
    challenges: &[String],
    disabled_schemes: &HashSet<String>,
) -> Option<(AuthScheme, Challenge)> {
    let mut schemes: HashMap<String, Challenge> = HashMap::new();
    for raw in challenges {
        let challenge = parse_challenge(raw);
        if challenge.scheme.is_empty() || disabled_schemes.contains(&challenge.scheme) {
            continue;
        }
        if schemes.contains_key(&challenge.scheme) {
            blaze_util::unported!(
                "AuthenticationHeader.parse",
                "two Proxy-Authenticate challenges for the scheme {}",
                challenge.scheme
            );
        }
        schemes.insert(challenge.scheme.clone(), challenge);
    }
    for name in ["negotiate", "kerberos"] {
        if schemes.contains_key(name) {
            // The JDK asks GSS-API whether it can produce a token for this proxy (Kerberos
            // configuration, ticket cache); without one it goes on to the next scheme.
            blaze_util::unported!(
                "NegotiateAuthentication.isSupported",
                "proxy challenge for {name}"
            );
        }
    }
    if let Some(c) = schemes.remove("digest") {
        return Some((AuthScheme::Digest, c));
    }
    if let Some(c) = schemes.remove("ntlm") {
        return Some((AuthScheme::Ntlm, c));
    }
    schemes.remove("basic").map(|c| (AuthScheme::Basic, c))
}

/// `BasicAuthentication`'s header value: `user + ":" + password` in ISO-8859-1 (a char above
/// U+00FF becomes `?`), or in UTF-8 when the challenge has `charset="UTF-8"`.
// Java: sun/net/www/protocol/http/BasicAuthentication.java BasicAuthentication.<init> (JDK 25.0.2)
fn basic_header_value(pw: &PasswordAuthentication, is_utf8: bool) -> String {
    use base64::Engine as _;
    let plain = format!("{}:{}", pw.user_name, pw.password);
    let bytes = if is_utf8 {
        plain.into_bytes()
    } else {
        latin1_encode_lossy(&plain).into_owned()
    };
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// The JVM-wide state of one Bazel server: system properties set at run time, the default
/// `Authenticator`, the authentication cache, and `HttpURLConnection`'s statics.
#[derive(Debug, Default)]
pub struct JvmNetState {
    /// System properties set by Bazel at run time (none are set on the command line).
    system_properties: Mutex<HashMap<String, String>>,
    /// Initialised by the first `HttpURLConnection` (its class initialisation).
    http_url_connection_statics: OnceLock<HttpUrlConnectionStatics>,
    /// `Authenticator.getDefault()` as ProxyHelper sets it: the proxy credentials.
    default_authenticator: Mutex<Option<PasswordAuthentication>>,
    /// `AuthCacheImpl`: proxy answers by (host, port, realm, scheme) and, for preemptive use, by
    /// (host, port).
    auth_cache: Mutex<AuthCache>,
}

#[derive(Debug, Default)]
struct AuthCache {
    by_realm: HashMap<(String, i32, String, AuthScheme), AuthenticationInfo>,
    by_server: HashMap<(String, i32), AuthenticationInfo>,
}

impl JvmNetState {
    /// A fresh JVM (tests).
    pub fn new() -> Self {
        JvmNetState::default()
    }

    /// The state of this process (Bazel's server JVM).
    pub fn global() -> &'static JvmNetState {
        static GLOBAL: OnceLock<JvmNetState> = OnceLock::new();
        GLOBAL.get_or_init(JvmNetState::new)
    }

    /// `System.getProperties().putIfAbsent(key, value)`.
    pub fn put_property_if_absent(&self, key: &str, value: &str) {
        self.system_properties
            .lock()
            .unwrap()
            .entry(key.to_string())
            .or_insert_with(|| value.to_string());
    }

    /// `NetProperties.get`: the system property, else `conf/net.properties`.
    fn net_property(&self, key: &str) -> Option<String> {
        self.system_properties
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .or_else(|| net_properties_default(key).map(str::to_string))
    }

    /// ProxyHelper's one-time setup when it first sees proxy credentials: the default
    /// `Authenticator` (proxy requests only) and `enableBasicAuthTunneling`. The first caller
    /// wins; later calls change nothing.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/ProxyHelper.java:312 ProxyHelper.createProxyInfo
    pub fn install_proxy_authenticator(&self, user_name_unicode: &str, password_unicode: &str) {
        let mut authenticator = self.default_authenticator.lock().unwrap();
        if authenticator.is_none() {
            // PORT: the previous default `Authenticator`, to which Bazel's delegates non-proxy
            // requests, is always null: nothing else in Bazel sets one.
            *authenticator = Some(PasswordAuthentication {
                user_name: user_name_unicode.to_string(),
                password: password_unicode.to_string(),
            });
            // Java: ProxyHelper.java:382 ProxyHelper.enableBasicAuthTunneling
            self.put_property_if_absent(TUNNELING_DISABLED_SCHEMES, "");
        }
    }

    /// `HttpURLConnection`'s class initialisation, which happens when the first connection object
    /// is made (`URL.openConnection`). Idempotent.
    pub fn http_url_connection_class_init(&self) {
        self.statics();
    }

    fn statics(&self) -> &HttpUrlConnectionStatics {
        self.http_url_connection_statics.get_or_init(|| {
            let tunneling = self.net_property(TUNNELING_DISABLED_SCHEMES);
            let proxying = self.net_property(PROXYING_DISABLED_SCHEMES);
            HttpUrlConnectionStatics {
                disabled_tunneling_schemes: schemes_list_to_set(tunneling.as_deref()),
                disabled_proxying_schemes: schemes_list_to_set(proxying.as_deref()),
            }
        })
    }

    /// `Authenticator.requestPasswordAuthentication` for a proxy.
    fn request_password_authentication(&self) -> Option<PasswordAuthentication> {
        self.default_authenticator.lock().unwrap().clone()
    }

    /// `setPreemptiveProxyAuthentication`: an earlier successful answer for this proxy.
    pub fn preemptive_proxy_authentication(
        &self,
        proxy_host: &str,
        proxy_port: i32,
    ) -> Option<AuthenticationInfo> {
        self.auth_cache
            .lock()
            .unwrap()
            .by_server
            .get(&(proxy_host.to_string(), proxy_port))
            .cloned()
    }

    /// `AuthenticationInfo.addToCache`.
    pub fn add_to_cache(&self, info: &AuthenticationInfo) {
        let mut cache = self.auth_cache.lock().unwrap();
        cache.by_realm.insert(
            (
                info.host.clone(),
                info.port,
                info.realm.clone(),
                info.scheme,
            ),
            info.clone(),
        );
        cache
            .by_server
            .insert((info.host.clone(), info.port), info.clone());
    }

    /// `AuthenticationInfo.removeFromCache`.
    fn remove_from_cache(&self, info: &AuthenticationInfo) {
        let mut cache = self.auth_cache.lock().unwrap();
        let realm_key = (
            info.host.clone(),
            info.port,
            info.realm.clone(),
            info.scheme,
        );
        if cache.by_realm.get(&realm_key) == Some(info) {
            cache.by_realm.remove(&realm_key);
        }
        let server_key = (info.host.clone(), info.port);
        if cache.by_server.get(&server_key) == Some(info) {
            cache.by_server.remove(&server_key);
        }
    }

    /// `HttpURLConnection.resetProxyAuthentication`: after a 407, the answer just sent is taken
    /// out of the cache (Basic is never stale).
    pub fn reset_proxy_authentication(&self, proxy_authentication: Option<&AuthenticationInfo>) {
        if let Some(info) = proxy_authentication
            && info.scheme != AuthScheme::Ntlm
        {
            self.remove_from_cache(info);
        }
    }

    /// `HttpURLConnection.getHttpProxyAuthentication`: the answer to a 407 from the proxy at
    /// `proxy_host:proxy_port`, or `None` when the JDK has none (no usable challenge, no
    /// credentials). `tunnel` selects the disabled-scheme list (`CONNECT` or a request through
    /// the proxy).
    pub fn get_http_proxy_authentication(
        &self,
        challenges: &[String],
        proxy_host: &str,
        proxy_port: i32,
        tunnel: bool,
    ) -> Option<AuthenticationInfo> {
        let statics = self.statics();
        let disabled = if tunnel {
            &statics.disabled_tunneling_schemes
        } else {
            &statics.disabled_proxying_schemes
        };
        let (scheme, challenge) = select_challenge(challenges, disabled)?;
        let realm = challenge.find_value("realm").unwrap_or("").to_string();
        let cached = self
            .auth_cache
            .lock()
            .unwrap()
            .by_realm
            .get(&(proxy_host.to_string(), proxy_port, realm.clone(), scheme))
            .cloned();
        if cached.is_some() {
            return cached;
        }
        match scheme {
            AuthScheme::Basic => {
                let pw = self.request_password_authentication()?;
                let is_utf8 = challenge
                    .find_value("charset")
                    .is_some_and(|c| c.eq_ignore_ascii_case("UTF-8"));
                Some(AuthenticationInfo {
                    scheme,
                    host: proxy_host.to_string(),
                    port: proxy_port,
                    realm,
                    header_value: basic_header_value(&pw, is_utf8),
                })
            }
            // Recorded: without credentials the JDK sends nothing for Digest or NTLM.
            AuthScheme::Digest => {
                self.request_password_authentication()?;
                blaze_util::unported!(
                    "DigestAuthentication",
                    "proxy challenge {:?}",
                    challenge.params
                )
            }
            AuthScheme::Ntlm => {
                self.request_password_authentication()?;
                blaze_util::unported!("NTLMAuthentication", "proxy challenge for NTLM")
            }
            AuthScheme::Negotiate | AuthScheme::Kerberos => {
                unreachable!("select_challenge does not return {scheme:?}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_parameters_are_found_ignoring_case() {
        let c = parse_challenge(r#"Basic realm="a, b", Charset="UTF-8""#);
        assert_eq!(c.scheme, "basic");
        assert_eq!(c.find_value("realm"), Some("a, b"));
        assert_eq!(c.find_value("charset"), Some("UTF-8"));
        let c = parse_challenge("NTLM");
        assert_eq!(c.scheme, "ntlm");
        assert!(c.params.is_empty());
    }

    #[test]
    fn disabled_schemes_come_from_net_properties_until_the_property_is_set() {
        let s = JvmNetState::new();
        s.http_url_connection_class_init();
        assert!(s.statics().disabled_tunneling_schemes.contains("basic"));
        assert!(s.statics().disabled_proxying_schemes.is_empty());
        // Too late: the statics are read once.
        s.install_proxy_authenticator("u", "p");
        assert!(s.statics().disabled_tunneling_schemes.contains("basic"));

        let s = JvmNetState::new();
        s.install_proxy_authenticator("u", "p");
        s.http_url_connection_class_init();
        assert!(s.statics().disabled_tunneling_schemes.is_empty());
    }

    #[test]
    fn the_first_credentials_win() {
        let s = JvmNetState::new();
        s.install_proxy_authenticator("first", "1");
        s.install_proxy_authenticator("second", "2");
        let a = s
            .get_http_proxy_authentication(&["Basic realm=\"r\"".into()], "h", 1, false)
            .unwrap();
        assert_eq!(a.header_value(), "Basic Zmlyc3Q6MQ==");
    }

    #[test]
    fn basic_is_latin1_or_utf8_by_charset() {
        let pw = PasswordAuthentication {
            user_name: "é中".into(),
            password: "pé".into(),
        };
        // Recorded (tests/data/proxy/https-enc-raw, https-enc-raw-utf8).
        assert_eq!(basic_header_value(&pw, false), "Basic 6T86cOk=");
        assert_eq!(basic_header_value(&pw, true), "Basic w6nkuK06cMOp");
    }
}
