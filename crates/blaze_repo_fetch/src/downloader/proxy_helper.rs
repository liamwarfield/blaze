//! Ports of `ProxyHelper` and `ProxyInfo`.
//!
//! PORT: Java also consults JVM system properties (`http.proxyHost`, `http.nonProxyHosts`, ...);
//! Blaze has no JVM, so those sources are empty. The JVM-wide `Authenticator` used for HTTPS
//! CONNECT tunnelling is replaced by passing the credentials to the HTTP client's proxy.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::authandtls::basic_http_authentication_encode;
use crate::downloader::http_utils;
use crate::errors::IoException;
use crate::uri::JavaUri;

/// `java.net.Proxy.Type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyType {
    DIRECT,
    HTTP,
    SOCKS,
}

/// Container for proxy configuration including the proxy address and optional credentials.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/ProxyInfo.java:29 ProxyInfo
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyInfo {
    pub proxy_type: ProxyType,
    pub hostname: Option<String>,
    pub port: i32,
    username: Option<String>,
    password: Option<String>,
}

impl ProxyInfo {
    /// A ProxyInfo representing no proxy (direct connection).
    pub fn no_proxy() -> ProxyInfo {
        ProxyInfo {
            proxy_type: ProxyType::DIRECT,
            hostname: None,
            port: -1,
            username: None,
            password: None,
        }
    }

    /// Returns true if this proxy requires authentication.
    pub fn has_credentials(&self) -> bool {
        self.username.is_some() && self.password.is_some()
    }

    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    pub fn password(&self) -> Option<&str> {
        self.password.as_deref()
    }

    /// The value for the Proxy-Authorization header, or `None` if no authentication is needed.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/ProxyInfo.java:59 ProxyInfo.getProxyAuthorizationHeader
    pub fn get_proxy_authorization_header(&self) -> Option<String> {
        if !self.has_credentials() {
            return None;
        }
        Some(basic_http_authentication_encode(
            self.username.as_deref().unwrap(),
            self.password.as_deref().unwrap(),
        ))
    }

    /// Java `Proxy.toString()`: `DIRECT`, or `HTTP @ host/<unresolved>:port` (the
    /// `InetSocketAddress` is kept unresolved; Java may print the resolved form).
    pub fn proxy_to_string(&self) -> String {
        match self.proxy_type {
            ProxyType::DIRECT => "DIRECT".to_string(),
            t => format!(
                "{:?} @ {}/<unresolved>:{}",
                t,
                self.hostname.as_deref().unwrap_or(""),
                self.port
            ),
        }
    }

    /// `Proxy.address()` as `host:port`, if not direct.
    pub fn address(&self) -> Option<String> {
        self.hostname
            .as_ref()
            .map(|h| format!("{}:{}", h, self.port))
    }
}

/// Helper class for setting up a proxy server for network communication.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/ProxyHelper.java:43 ProxyHelper
#[derive(Clone, Debug, Default)]
pub struct ProxyHelper {
    env: HashMap<String, String>,
}

static URL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(https?://|socks[45]?://)?(([^:@]+?)(?::([^@]+?))?@)?([^:]+)(?::([0-9]+))?/?$")
        .unwrap()
});

/// `s.split("\\s*,\\s*")` (Java `String.split(regex)` with limit 0): the separator is a comma
/// with the whitespace (`\s`: space, `\t`, `\n`, `\x0B`, `\f`, `\r`) around it, so the first
/// entry keeps its leading and the last entry its trailing whitespace; a separator at the start
/// gives a leading empty entry, and trailing empty entries are dropped.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/ProxyHelper.java:85 ProxyHelper.createProxyIfNeeded
// PORT: JDK `String.split(String regex)` (limit 0) for this one regex.
fn java_split_comma(s: &str) -> Vec<&str> {
    fn is_java_whitespace(b: u8) -> bool {
        matches!(b, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
    }
    let bytes = s.as_bytes();
    let mut entries: Vec<&str> = Vec::new();
    // End of the previous match: where the next entry starts.
    let mut index = 0;
    let mut comma = index;
    while let Some(offset) = s[comma..].find(',') {
        let comma_at = comma + offset;
        // The match starts at the first whitespace of the run just before the comma (the regex
        // engine tries each start position from the left), but not inside the previous match.
        let mut match_start = comma_at;
        while match_start > index && is_java_whitespace(bytes[match_start - 1]) {
            match_start -= 1;
        }
        let mut match_end = comma_at + 1;
        while match_end < bytes.len() && is_java_whitespace(bytes[match_end]) {
            match_end += 1;
        }
        entries.push(&s[index..match_start]);
        index = match_end;
        comma = match_end;
    }
    if entries.is_empty() {
        // If no match was found, return this.
        return vec![s];
    }
    // Add remaining segment.
    entries.push(&s[index..]);
    // Construct result: trailing empty strings are removed.
    while entries.last() == Some(&"") {
        entries.pop();
    }
    entries
}

fn non_empty(s: Option<&String>) -> Option<&String> {
    s.filter(|s| !s.is_empty())
}

/// Java `URLDecoder.decode(s, UTF_8)`.
fn url_decode(s: &str) -> Result<String, IoException> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' => {
                let v = bytes
                    .get(i + 1..i + 3)
                    .and_then(|h| std::str::from_utf8(h).ok())
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                match v {
                    Some(v) => out.push(v),
                    None => {
                        panic!(
                            "IllegalArgumentException: URLDecoder: Illegal hex characters in escape (%) pattern"
                        )
                    }
                }
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    // PORT: byte-strings (percent-decoded proxy credentials; URLDecoder.decode(.., UTF_8) also replaces malformed input)
    Ok(String::from_utf8_lossy(&out).into_owned())
}

impl ProxyHelper {
    /// Creates a new instance; `env` is the client environment to check for proxy settings.
    pub fn new(env: HashMap<String, String>) -> Self {
        ProxyHelper { env }
    }

    /// Sets up a proxy for `requested_url` if `HTTP_PROXY`/`HTTPS_PROXY` (or lowercase) are
    /// set, honouring `no_proxy`/`NO_PROXY`.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/ProxyHelper.java:78 ProxyHelper.createProxyIfNeeded
    pub fn create_proxy_if_needed(
        &self,
        requested_url: &JavaUri,
    ) -> Result<ProxyInfo, IoException> {
        // Check no_proxy/NO_PROXY environment variables
        let no_proxy_url = non_empty(self.env.get("no_proxy")).or_else(|| self.env.get("NO_PROXY"));
        if let Some(no_proxy_url) = non_empty(no_proxy_url) {
            let entries = java_split_comma(no_proxy_url);
            // PORT: Java dereferences getHost() and would throw NullPointerException.
            let requested_host = requested_url.get_host().unwrap_or("");
            for entry in entries {
                if entry.starts_with('.') {
                    // This entry applies to sub-domains only.
                    if requested_host.ends_with(entry) {
                        return Ok(ProxyInfo::no_proxy());
                    }
                } else if requested_host == entry
                    || requested_host.ends_with(&format!(".{}", entry))
                {
                    // This entry applies to the literal hostname and sub-domains.
                    return Ok(ProxyInfo::no_proxy());
                }
            }
        }
        let proxy_address = if http_utils::is_protocol(requested_url, "https") {
            self.env
                .get("https_proxy")
                .or_else(|| self.env.get("HTTPS_PROXY"))
        } else if http_utils::is_protocol(requested_url, "http") {
            self.env
                .get("http_proxy")
                .or_else(|| self.env.get("HTTP_PROXY"))
        } else {
            None
        };
        create_proxy_info(proxy_address.map(String::as_str), None, None)
    }
}

/// `ProxyHelper.createProxy`.
pub fn create_proxy(proxy_address: Option<&str>) -> Result<ProxyInfo, IoException> {
    create_proxy_info(proxy_address, None, None)
}

/// Creates a [`ProxyInfo`] from a proxy address URL (possibly with embedded credentials).
// Java: com/google/devtools/build/lib/bazel/repository/downloader/ProxyHelper.java:228 ProxyHelper.createProxyInfo
pub fn create_proxy_info(
    proxy_address: Option<&str>,
    system_property_user: Option<&str>,
    system_property_password: Option<&str>,
) -> Result<ProxyInfo, IoException> {
    let Some(proxy_address) = proxy_address.filter(|s| !s.is_empty()) else {
        return Ok(ProxyInfo::no_proxy());
    };
    // Here there be dragons.
    let Some(m) = URL_PATTERN.captures(proxy_address) else {
        return Err(IoException::new(format!(
            "Proxy address {} is not a valid URL",
            proxy_address
        )));
    };
    let protocol = m.get(1).map(|x| x.as_str());
    let id_and_password = m.get(2).map(|x| x.as_str());
    let url_username = m.get(3).map(|x| x.as_str());
    let url_password = m.get(4).map(|x| x.as_str());
    let hostname = m.get(5).map(|x| x.as_str()).unwrap();
    let port_raw = m.get(6).map(|x| x.as_str());

    let clean_proxy_address = match id_and_password {
        Some(idp) => proxy_address.replace(idp, ""),
        None => proxy_address.to_string(),
    };
    let (proxy_type, default_port) = match protocol {
        Some("https://") => (ProxyType::HTTP, 443),
        Some("http://") | None => (ProxyType::HTTP, 80),
        Some("socks://") | Some("socks4://") | Some("socks5://") => (ProxyType::SOCKS, 1080),
        Some(_) => {
            return Err(IoException::new(format!(
                "Invalid proxy protocol for {}",
                clean_proxy_address
            )));
        }
    };
    let port = match port_raw {
        Some(p) => p.parse::<i32>().map_err(|_| {
            IoException::new(format!("Error parsing proxy port: {}", clean_proxy_address))
        })?,
        None => default_port,
    };
    // Determine credentials: URL credentials take precedence over system properties
    let (username, password) = match url_username {
        Some(u) => {
            let Some(p) = url_password else {
                return Err(IoException::new(format!(
                    "No password given for proxy {}",
                    clean_proxy_address
                )));
            };
            // We need to make sure the proxy credentials are not url encoded.
            (Some(url_decode(u)?), Some(url_decode(p)?))
        }
        None => match (system_property_user, system_property_password) {
            (Some(u), Some(p)) => (Some(u.to_string()), Some(p.to_string())),
            _ => (None, None),
        },
    };
    Ok(ProxyInfo {
        proxy_type,
        hostname: Some(hostname.to_string()),
        port,
        username,
        password,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values: Java's `String.split("\\s*,\\s*")`, which ProxyHelper.java:85 applies to
    // `no_proxy`, printed by OpenJDK 25.0.5 (the oracle embeds JDK 25) for each input below.
    #[test]
    fn java_split_comma_trims_around_separators() {
        assert_eq!(java_split_comma("a , b,c,,"), vec!["a", "b", "c"]);
        assert_eq!(java_split_comma(" a"), vec![" a"]);
        assert_eq!(java_split_comma(" a , b ,c "), vec![" a", "b", "c "]);
        assert_eq!(java_split_comma(", a"), vec!["", "a"]);
        assert_eq!(java_split_comma(" , a"), vec!["", "a"]);
        assert_eq!(java_split_comma("a , , b"), vec!["a", "", "b"]);
        assert_eq!(java_split_comma("a\t,\nb"), vec!["a", "b"]);
        assert_eq!(java_split_comma(","), Vec::<&str>::new());
        assert_eq!(java_split_comma("a b,c"), vec!["a b", "c"]);
    }

    /// `no_proxy=" example.com"` keeps the leading space, so the host does not match and the
    /// proxy is used (ProxyHelper.java:85-99).
    #[test]
    fn no_proxy_entry_with_leading_whitespace_does_not_match() {
        let mut env = HashMap::new();
        env.insert("no_proxy".to_owned(), " example.com".to_owned());
        env.insert(
            "https_proxy".to_owned(),
            "http://proxy.local:3128".to_owned(),
        );
        let helper = ProxyHelper::new(env);
        let url = JavaUri::create("https://example.com/file").unwrap();
        let info = helper.create_proxy_if_needed(&url).unwrap();
        assert_eq!(info.proxy_type, ProxyType::HTTP);
    }
}
