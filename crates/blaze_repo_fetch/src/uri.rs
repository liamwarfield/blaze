//! A port of the parts of `java.net.URI` the downloader relies on.
//!
//! Bazel keeps URLs as `java.net.URI`: `toString()` returns the original text (error messages
//! such as `Error downloading [<urls>] to ...` print it verbatim), and the getters split it per
//! RFC 2396 with Java's quirks (`getHost()` is `null` for a registry-based authority such as a
//! host name containing `_`; `getPath()` and `getUserInfo()` are percent-decoded).
//!
//! PORT: character validation is a simplified version of `URI.Parser` (illegal characters are
//! rejected with Java's message shape `Illegal character in <component> at index <i>: <input>`);
//! IPv6 literals are accepted without validating their contents.

use std::fmt;
use std::hash::{Hash, Hasher};

use blaze_util::string_encoding::java_new_string_utf8;

/// Java `URISyntaxException` (the message is `getMessage()`: `<reason>: <input>` or
/// `<reason> at index <i>: <input>`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UriSyntaxException {
    pub message: String,
}

impl fmt::Display for UriSyntaxException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for UriSyntaxException {}

/// Port of `java.net.URI` (immutable).
#[derive(Clone, Debug)]
pub struct JavaUri {
    string: String,
    scheme: Option<String>,
    raw_ssp: String,
    raw_authority: Option<String>,
    raw_user_info: Option<String>,
    host: Option<String>,
    port: i32,
    raw_path: Option<String>,
    raw_query: Option<String>,
    raw_fragment: Option<String>,
}

/// `index` is a byte offset into `input`; Java's index counts chars (UTF-16 units).
fn fail(reason: &str, input: &str, index: Option<usize>) -> UriSyntaxException {
    UriSyntaxException {
        message: match index {
            Some(i) => format!(
                "{} at index {}: {}",
                reason,
                input.get(..i).map_or(i, |p| p.encode_utf16().count()),
                input
            ),
            None => format!("{}: {}", reason, input),
        },
    }
}

fn is_alpha(c: char) -> bool {
    c.is_ascii_alphabetic()
}

fn is_alnum(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

/// RFC 2396 `unreserved | reserved | escaped | other`, minus the delimiters each component
/// excludes. Returns the index of the first illegal character, if any.
fn check_chars(s: &str, offset: usize, extra_disallowed: &str) -> Option<usize> {
    let bytes: Vec<(usize, char)> = s.char_indices().collect();
    let mut i = 0;
    while i < bytes.len() {
        let (pos, c) = bytes[i];
        if c == '%' {
            // escaped = "%" hex hex
            let ok = bytes
                .get(i + 1)
                .map(|(_, c)| c.is_ascii_hexdigit())
                .unwrap_or(false)
                && bytes
                    .get(i + 2)
                    .map(|(_, c)| c.is_ascii_hexdigit())
                    .unwrap_or(false);
            if !ok {
                return Some(offset + pos);
            }
            i += 3;
            continue;
        }
        let legal = if c.is_ascii() {
            is_alnum(c) || "-_.!~*'();/?:@&=+$,[]".contains(c)
        } else {
            // "other": non-US-ASCII characters that are neither control nor space characters.
            !c.is_control() && !c.is_whitespace()
        };
        if !legal || extra_disallowed.contains(c) {
            return Some(offset + pos);
        }
        i += 1;
    }
    None
}

/// Percent-decodes a raw component (Java `URI.decode`).
fn decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    // java.net.URI decodes the escaped bytes as UTF-8, replacing malformed input. The other
    // chars are copied, so their UTF-8 bytes decode to themselves.
    java_new_string_utf8(&out)
}

/// RFC 2396 hostname (with Java's rule that the top label starts with a letter) or IPv4.
fn is_server_host(host: &str) -> bool {
    if host.starts_with('[') {
        return host.ends_with(']') && host.len() > 2;
    }
    if host.is_empty() {
        return false;
    }
    let labels: Vec<&str> = host.split('.').collect();
    // IPv4address = 1*digit "." 1*digit "." 1*digit "." 1*digit
    if labels.len() == 4
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.len() <= 3 && l.chars().all(|c| c.is_ascii_digit()))
    {
        return true;
    }
    // A trailing dot is allowed ("example.com.").
    let labels: &[&str] = if labels.len() > 1 && labels.last() == Some(&"") {
        &labels[..labels.len() - 1]
    } else {
        &labels[..]
    };
    for l in labels {
        if l.is_empty()
            || !l.chars().all(|c| is_alnum(c) || c == '-')
            || !is_alnum(l.chars().next().unwrap())
            || !is_alnum(l.chars().last().unwrap())
        {
            return false;
        }
    }
    is_alpha(labels.last().unwrap().chars().next().unwrap())
}

impl JavaUri {
    /// `URI.create` / `new URI(String)`.
    pub fn create(input: &str) -> Result<JavaUri, UriSyntaxException> {
        Self::parse(input, false)
    }

    fn parse(input: &str, require_server_authority: bool) -> Result<JavaUri, UriSyntaxException> {
        let mut uri = JavaUri {
            string: input.to_string(),
            scheme: None,
            raw_ssp: String::new(),
            raw_authority: None,
            raw_user_info: None,
            host: None,
            port: -1,
            raw_path: None,
            raw_query: None,
            raw_fragment: None,
        };
        if input.is_empty() {
            // new URI("") is a valid empty relative reference.
            uri.raw_path = Some(String::new());
            return Ok(uri);
        }
        // scheme
        let delim = input.find([':', '/', '?', '#']);
        let mut rest_start = 0;
        if let Some(d) = delim
            && input.as_bytes()[d] == b':'
        {
            if d == 0 {
                return Err(fail("Expected scheme name", input, Some(0)));
            }
            let scheme = &input[..d];
            let mut cs = scheme.chars();
            let first = cs.next().unwrap();
            if !is_alpha(first) {
                return Err(fail("Illegal character in scheme name", input, Some(0)));
            }
            if let Some(p) = scheme
                .char_indices()
                .find(|(_, c)| !(is_alnum(*c) || "+-.".contains(*c)))
                .map(|(i, _)| i)
            {
                return Err(fail("Illegal character in scheme name", input, Some(p)));
            }
            uri.scheme = Some(scheme.to_string());
            rest_start = d + 1;
            if rest_start == input.len() {
                return Err(fail(
                    "Expected scheme-specific part",
                    input,
                    Some(rest_start),
                ));
            }
        }
        let rest = &input[rest_start..];
        // fragment
        let (body, fragment) = match rest.find('#') {
            Some(h) => (&rest[..h], Some(&rest[h + 1..])),
            None => (rest, None),
        };
        if let Some(f) = fragment {
            if let Some(i) = check_chars(f, rest_start + body.len() + 1, "#") {
                return Err(fail("Illegal character in fragment", input, Some(i)));
            }
            uri.raw_fragment = Some(f.to_string());
        }
        uri.raw_ssp = body.to_string();
        if uri.scheme.is_some() && !body.starts_with('/') {
            // Opaque URI.
            if let Some(i) = check_chars(body, rest_start, "") {
                return Err(fail("Illegal character in opaque part", input, Some(i)));
            }
            return Ok(uri);
        }
        // Hierarchical.
        let (hier, query) = match body.find('?') {
            Some(q) => (&body[..q], Some(&body[q + 1..])),
            None => (body, None),
        };
        if let Some(q) = query {
            if let Some(i) = check_chars(q, rest_start + hier.len() + 1, "") {
                return Err(fail("Illegal character in query", input, Some(i)));
            }
            uri.raw_query = Some(q.to_string());
        }
        let path_start;
        let path = if let Some(after) = hier.strip_prefix("//") {
            let auth_end = after.find('/').unwrap_or(after.len());
            let authority = &after[..auth_end];
            let auth_offset = rest_start + 2;
            if let Some(i) = check_chars(authority, auth_offset, "/?#") {
                return Err(fail("Illegal character in authority", input, Some(i)));
            }
            if !authority.is_empty() {
                uri.raw_authority = Some(authority.to_string());
                uri.parse_server_authority(
                    authority,
                    input,
                    auth_offset,
                    require_server_authority,
                )?;
            } else if uri.scheme.is_some() && after[auth_end..].is_empty() {
                return Err(fail("Expected authority", input, Some(auth_offset)));
            }
            path_start = auth_offset + auth_end;
            &after[auth_end..]
        } else {
            path_start = rest_start;
            hier
        };
        if let Some(i) = check_chars(path, path_start, "?") {
            return Err(fail("Illegal character in path", input, Some(i)));
        }
        uri.raw_path = Some(path.to_string());
        Ok(uri)
    }

    fn parse_server_authority(
        &mut self,
        authority: &str,
        input: &str,
        offset: usize,
        require_server_authority: bool,
    ) -> Result<(), UriSyntaxException> {
        let (user_info, host_port) = match authority.rfind('@') {
            Some(a) => (Some(&authority[..a]), &authority[a + 1..]),
            None => (None, authority),
        };
        let (host, port) = if host_port.starts_with('[') {
            match host_port.find(']') {
                Some(e) => (&host_port[..=e], &host_port[e + 1..]),
                None => (host_port, ""),
            }
        } else {
            match host_port.rfind(':') {
                Some(c) => (&host_port[..c], &host_port[c..]),
                None => (host_port, ""),
            }
        };
        let port_ok = port.is_empty()
            || (port.starts_with(':') && port[1..].chars().all(|c| c.is_ascii_digit()));
        let user_info_ok = user_info.is_none_or(|u| !u.contains('@'));
        if is_server_host(host) && port_ok && user_info_ok {
            self.raw_user_info = user_info.map(str::to_string);
            self.host = Some(host.to_string());
            if port.len() > 1 {
                self.port = match port[1..].parse::<i32>() {
                    Ok(p) => p,
                    Err(_) => {
                        return Err(fail(
                            "Malformed port number",
                            input,
                            Some(offset + authority.len() - port.len() + 1),
                        ));
                    }
                };
            }
            Ok(())
        } else if require_server_authority {
            Err(fail("Illegal character in hostname", input, Some(offset)))
        } else {
            // Registry-based authority: host, port and user info stay undefined.
            Ok(())
        }
    }

    /// The 7-argument `URI` constructor: builds the string (quoting illegal characters) and
    /// parses it with a server-based authority required.
    pub fn from_components(
        scheme: Option<&str>,
        user_info: Option<&str>,
        host: Option<&str>,
        port: i32,
        path: Option<&str>,
        query: Option<&str>,
        fragment: Option<&str>,
    ) -> Result<JavaUri, UriSyntaxException> {
        // Java: java/net/URI.java URI.checkPath
        if scheme.is_some()
            && let Some(p) = path
            && !p.is_empty()
            && !p.starts_with('/')
        {
            return Err(UriSyntaxException {
                message: format!("Relative path in absolute URI: {}", p),
            });
        }
        let mut sb = String::new();
        if let Some(s) = scheme {
            sb.push_str(s);
            sb.push(':');
        }
        if let Some(h) = host {
            sb.push_str("//");
            if let Some(u) = user_info {
                sb.push_str(&quote(u, ";:&=+$,"));
                sb.push('@');
            }
            let needs_brackets = h.contains(':') && !h.starts_with('[') && !h.ends_with(']');
            if needs_brackets {
                sb.push('[');
            }
            sb.push_str(h);
            if needs_brackets {
                sb.push(']');
            }
            if port != -1 {
                sb.push(':');
                sb.push_str(&port.to_string());
            }
        }
        if let Some(p) = path {
            sb.push_str(&quote(p, "/@;:&=+$,"));
        }
        if let Some(q) = query {
            sb.push('?');
            sb.push_str(&quote(q, ";/?:@&=+$,[]"));
        }
        if let Some(f) = fragment {
            sb.push('#');
            sb.push_str(&quote(f, ";/?:@&=+$,[]"));
        }
        Self::parse(&sb, true)
    }

    /// `URI.getScheme()`.
    pub fn get_scheme(&self) -> Option<&str> {
        self.scheme.as_deref()
    }

    /// `URI.isOpaque()`.
    pub fn is_opaque(&self) -> bool {
        self.raw_path.is_none()
    }

    /// `URI.getRawSchemeSpecificPart()`.
    pub fn get_raw_scheme_specific_part(&self) -> &str {
        &self.raw_ssp
    }

    /// `URI.getSchemeSpecificPart()`.
    pub fn get_scheme_specific_part(&self) -> String {
        decode(&self.raw_ssp)
    }

    /// `URI.getRawAuthority()`.
    pub fn get_raw_authority(&self) -> Option<&str> {
        self.raw_authority.as_deref()
    }

    /// `URI.getUserInfo()` (decoded).
    pub fn get_user_info(&self) -> Option<String> {
        self.raw_user_info.as_deref().map(decode)
    }

    /// `URI.getRawUserInfo()`.
    pub fn get_raw_user_info(&self) -> Option<&str> {
        self.raw_user_info.as_deref()
    }

    /// `URI.getHost()`.
    pub fn get_host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// `URI.getPort()`; -1 if undefined.
    pub fn get_port(&self) -> i32 {
        self.port
    }

    /// `URI.getPath()` (decoded); `None` for an opaque URI.
    pub fn get_path(&self) -> Option<String> {
        self.raw_path.as_deref().map(decode)
    }

    /// `URI.getRawPath()`.
    pub fn get_raw_path(&self) -> Option<&str> {
        self.raw_path.as_deref()
    }

    /// `URI.getQuery()` (decoded).
    pub fn get_query(&self) -> Option<String> {
        self.raw_query.as_deref().map(decode)
    }

    /// `URI.getRawQuery()`.
    pub fn get_raw_query(&self) -> Option<&str> {
        self.raw_query.as_deref()
    }

    /// `URI.getFragment()` (decoded).
    pub fn get_fragment(&self) -> Option<String> {
        self.raw_fragment.as_deref().map(decode)
    }

    /// `URI.getRawFragment()`.
    pub fn get_raw_fragment(&self) -> Option<&str> {
        self.raw_fragment.as_deref()
    }

    /// `URI.toString()`: the original text.
    pub fn as_str(&self) -> &str {
        &self.string
    }

    /// Normalized form for `equals`/`hashCode`: scheme and host compare case-insensitively.
    fn key(&self) -> (Option<String>, String, Option<String>) {
        let ssp = match (&self.host, &self.raw_authority) {
            (Some(h), Some(a)) => {
                let lowered = a.replacen(h.as_str(), &h.to_ascii_lowercase(), 1);
                self.raw_ssp.replacen(a.as_str(), &lowered, 1)
            }
            _ => self.raw_ssp.clone(),
        };
        (
            self.scheme.as_ref().map(|s| s.to_ascii_lowercase()),
            ssp,
            self.raw_fragment.clone(),
        )
    }
}

/// Java `URI.quote`: percent-encodes characters that are neither unreserved, in `allowed`, nor
/// "other" (non-ASCII, non-space, non-control).
fn quote(s: &str, allowed: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii() {
            if is_alnum(c) || "-_.!~*'()".contains(c) || allowed.contains(c) {
                out.push(c);
            } else {
                out.push_str(&format!("%{:02X}", c as u32));
            }
        } else if c.is_control() || c.is_whitespace() {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{:02X}", b));
            }
        } else {
            out.push(c);
        }
    }
    out
}

impl PartialEq for JavaUri {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for JavaUri {}

impl Hash for JavaUri {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

impl fmt::Display for JavaUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.string)
    }
}

/// Java `List<URI>.toString()`: `[a, b]`.
pub fn uri_list_to_string(uris: &[JavaUri]) -> String {
    let parts: Vec<&str> = uris.iter().map(JavaUri::as_str).collect();
    format!("[{}]", parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_url() {
        let u =
            JavaUri::create("https://user:pw@example.com:8080/a%20b/c.tar.gz?x=1#frag").unwrap();
        assert_eq!(u.get_scheme(), Some("https"));
        assert_eq!(u.get_host(), Some("example.com"));
        assert_eq!(u.get_port(), 8080);
        assert_eq!(u.get_user_info().as_deref(), Some("user:pw"));
        assert_eq!(u.get_path().as_deref(), Some("/a b/c.tar.gz"));
        assert_eq!(u.get_query().as_deref(), Some("x=1"));
        assert_eq!(u.get_fragment().as_deref(), Some("frag"));
        assert!(!u.is_opaque());
    }

    #[test]
    fn underscore_host_is_registry_based() {
        let u = JavaUri::create("http://my_host/x").unwrap();
        assert_eq!(u.get_host(), None);
        assert_eq!(u.get_raw_authority(), Some("my_host"));
    }

    #[test]
    fn opaque_file_uri() {
        let u = JavaUri::create("file:../archive.tgz").unwrap();
        assert!(u.is_opaque());
        assert_eq!(u.get_path(), None);
        assert_eq!(u.get_raw_scheme_specific_part(), "../archive.tgz");
    }

    #[test]
    fn rejects_space() {
        let e = JavaUri::create("http://a.com/b c").unwrap_err();
        assert_eq!(
            e.message,
            "Illegal character in path at index 14: http://a.com/b c"
        );
    }

    // Byte-strings review 2026-11-20 S2-6. Expected values from `new java.net.URI(s)` (JDK 25):
    // the index counts UTF-16 units. The internal string of `https://h/é x` has é as the two
    // chars C3 A9 (index 12, not the UTF-8 byte offset 14); 🐷 is two units.
    #[test]
    fn syntax_error_index_counts_utf16_units() {
        let e = JavaUri::create("https://h/\u{c3}\u{a9} x").unwrap_err();
        assert_eq!(
            e.message,
            "Illegal character in path at index 12: https://h/\u{c3}\u{a9} x"
        );
        let e = JavaUri::create("http://a/🐷 x").unwrap_err();
        assert_eq!(
            e.message,
            "Illegal character in path at index 11: http://a/🐷 x"
        );
        let e = JavaUri::create("f\u{c3}\u{a9}:///x").unwrap_err();
        assert_eq!(
            e.message,
            "Illegal character in scheme name at index 1: f\u{c3}\u{a9}:///x"
        );
    }

    #[test]
    fn equality_ignores_scheme_and_host_case() {
        assert_eq!(
            JavaUri::create("HTTP://Example.COM/x").unwrap(),
            JavaUri::create("http://example.com/x").unwrap()
        );
    }
}
