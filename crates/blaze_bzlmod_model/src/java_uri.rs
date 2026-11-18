//! The subset of `java.net.URI` (RFC 2396 parsing) that registries use: scheme, raw authority,
//! path, query, and `toString` of the original input.

use std::fmt;

/// `java.net.URISyntaxException`. `get_message` is Java's `reason [at index N]: input`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UriSyntaxException {
    pub input: String,
    pub reason: String,
    pub index: i32,
}

impl UriSyntaxException {
    pub fn new(input: impl Into<String>, reason: impl Into<String>) -> Self {
        UriSyntaxException {
            input: input.into(),
            reason: reason.into(),
            index: -1,
        }
    }

    pub fn get_message(&self) -> String {
        if self.index > -1 {
            format!("{} at index {}: {}", self.reason, self.index, self.input)
        } else {
            format!("{}: {}", self.reason, self.input)
        }
    }
}

impl fmt::Display for UriSyntaxException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_message())
    }
}

impl std::error::Error for UriSyntaxException {}

/// A parsed URI (`java.net.URI`).
// PORT: parses with the RFC 2396 component split and rejects the characters Java rejects in the
// common cases; IPv6 literals and server-based authority validation are not checked.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JavaUri {
    string: String,
    scheme: Option<String>,
    authority: Option<String>,
    path: Option<String>,
    query: Option<String>,
    fragment: Option<String>,
}

fn is_illegal(c: char) -> bool {
    (c as u32) < 0x20
        || c == '\u{7f}'
        || matches!(
            c,
            ' ' | '"' | '<' | '>' | '\\' | '^' | '`' | '{' | '|' | '}'
        )
}

impl JavaUri {
    /// `new URI(str)`.
    pub fn new(s: &str) -> Result<JavaUri, UriSyntaxException> {
        let err = |reason: &str, index: usize| UriSyntaxException {
            input: s.to_string(),
            reason: reason.to_string(),
            index: index as i32,
        };
        if s.is_empty() {
            // An empty string is a valid relative URI with an empty path.
            return Ok(JavaUri {
                string: String::new(),
                scheme: None,
                authority: None,
                path: Some(String::new()),
                query: None,
                fragment: None,
            });
        }
        let mut rest_start = 0;
        let mut scheme = None;
        if let Some(colon) = s.find(':') {
            let before = &s[..colon];
            if !before.is_empty() && !before.contains(['/', '?', '#']) {
                let b = before.as_bytes();
                if !b[0].is_ascii_alphabetic() {
                    return Err(err("Illegal character in scheme name", 0));
                }
                if let Some(i) = b
                    .iter()
                    .position(|c| !(c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.')))
                {
                    return Err(err("Illegal character in scheme name", i));
                }
                scheme = Some(before.to_string());
                rest_start = colon + 1;
            } else if before.is_empty() {
                return Err(err("Expected scheme name", 0));
            }
        }
        let rest = &s[rest_start..];
        // Fragment.
        let (ssp_and_more, fragment) = match rest.find('#') {
            Some(i) => (&rest[..i], Some(rest[i + 1..].to_string())),
            None => (rest, None),
        };
        if scheme.is_some() && ssp_and_more.is_empty() {
            return Err(err("Expected scheme-specific part", s.len()));
        }
        // Opaque URI: a scheme whose scheme-specific part does not start with '/'.
        if scheme.is_some() && !ssp_and_more.starts_with('/') {
            if let Some((i, _)) = ssp_and_more.char_indices().find(|(_, c)| is_illegal(*c)) {
                return Err(err("Illegal character in opaque part", rest_start + i));
            }
            return Ok(JavaUri {
                string: s.to_string(),
                scheme,
                authority: None,
                path: None,
                query: None,
                fragment,
            });
        }
        let mut pos = 0;
        let mut authority = None;
        if let Some(after_slashes) = ssp_and_more.strip_prefix("//") {
            let a_end = after_slashes
                .find(['/', '?'])
                .map(|i| i + 2)
                .unwrap_or(ssp_and_more.len());
            let a = &ssp_and_more[2..a_end];
            if let Some((i, _)) = a.char_indices().find(|(_, c)| is_illegal(*c)) {
                return Err(err("Illegal character in authority", rest_start + 2 + i));
            }
            authority = if a.is_empty() {
                None
            } else {
                Some(a.to_string())
            };
            pos = a_end;
        }
        let after = &ssp_and_more[pos..];
        let (path, query) = match after.find('?') {
            Some(i) => (&after[..i], Some(after[i + 1..].to_string())),
            None => (after, None),
        };
        if let Some((i, _)) = path.char_indices().find(|(_, c)| is_illegal(*c)) {
            return Err(err("Illegal character in path", rest_start + pos + i));
        }
        if let Some(q) = &query
            && let Some((i, _)) = q.char_indices().find(|(_, c)| is_illegal(*c))
        {
            return Err(err(
                "Illegal character in query",
                rest_start + pos + path.len() + 1 + i,
            ));
        }
        Ok(JavaUri {
            string: s.to_string(),
            scheme,
            authority,
            path: Some(path.to_string()),
            query,
            fragment,
        })
    }

    pub fn get_scheme(&self) -> Option<&str> {
        self.scheme.as_deref()
    }

    pub fn get_raw_authority(&self) -> Option<&str> {
        self.authority.as_deref()
    }

    pub fn get_raw_path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// `getPath()`: the decoded path (percent escapes decoded), `None` for opaque URIs.
    pub fn get_path(&self) -> Option<String> {
        self.path.as_deref().map(percent_decode)
    }

    pub fn get_raw_query(&self) -> Option<&str> {
        self.query.as_deref()
    }

    pub fn get_raw_fragment(&self) -> Option<&str> {
        self.fragment.as_deref()
    }
}

impl fmt::Display for JavaUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.string)
    }
}

/// `URI.decode`: decodes `%XX` escapes as UTF-8 (malformed sequences become U+FFFD).
fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
