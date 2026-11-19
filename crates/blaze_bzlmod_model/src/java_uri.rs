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

/// A char `java.net.URI`'s parser rejects in every component: an ASCII char outside the RFC 2396
/// classes, or an "other" char (above U+007F) that is an ISO control char or a space char
/// (`URI.Parser.scanEscape` accepts other chars only when `!Character.isSpaceChar(c) &&
/// !Character.isISOControl(c)`). Registry URLs are internal strings, so U+0080–U+009F (the
/// second byte of most UTF-8 sequences) and U+00A0 are rejected, as in Java.
fn is_illegal(c: char) -> bool {
    (c as u32) < 0x20
        || ('\u{7f}'..='\u{a0}').contains(&c)
        || matches!(
            c,
            ' ' | '"' | '<' | '>' | '\\' | '^' | '`' | '{' | '|' | '}'
        )
        // The other Character.isSpaceChar code points (Zs, Zl, Zp).
        || matches!(
            c,
            '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}'
                | '\u{205f}' | '\u{3000}'
        )
}

impl JavaUri {
    /// `new URI(str)`.
    pub fn new(s: &str) -> Result<JavaUri, UriSyntaxException> {
        // `index` is a byte offset into `s`; Java reports the UTF-16 index (one per char of an
        // internal string).
        let err = |reason: &str, index: usize| UriSyntaxException {
            input: s.to_string(),
            reason: reason.to_string(),
            index: s[..index].encode_utf16().count() as i32,
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

/// `URI.decode`: each run of `%XX` escapes is decoded as UTF-8 (malformed input becomes U+FFFD,
/// as the JDK's replacing decoder does); every other char is copied as it is.
// PORT: Java's parser has already rejected a `%` not followed by two hex digits; such a `%` is
// copied here.
fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let escape_at = |i: usize| {
        b.get(i) == Some(&b'%')
            && b.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
            && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit)
    };
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if escape_at(i) {
            let mut run = Vec::new();
            while escape_at(i) {
                run.push(u8::from_str_radix(&s[i + 1..i + 3], 16).expect("hex digits"));
                i += 3;
            }
            out.push_str(&blaze_util::string_encoding::java_new_string_utf8(&run));
            continue;
        }
        let c = s[i..].chars().next().expect("char boundary");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values: `new URI(s)` / `getPath()` / `getMessage()` of a JDK 21 `java.net.URI`
    // (probe run on 2026-11-18), with internal strings (one char per byte) as input.

    fn path_units(s: &str) -> Vec<u32> {
        JavaUri::new(s)
            .unwrap()
            .get_path()
            .unwrap()
            .chars()
            .map(|c| c as u32)
            .collect()
    }

    fn message(s: &str) -> String {
        JavaUri::new(s).unwrap_err().get_message()
    }

    #[test]
    fn non_ascii_internal_chars_are_kept_in_the_path() {
        // "é" as an internal string.
        assert_eq!(
            path_units("file:///x\u{c3}\u{a9}"),
            [0x2f, 0x78, 0xc3, 0xa9]
        );
    }

    #[test]
    fn latin1_control_and_space_chars_are_illegal() {
        // "🐷" as an internal string: F0 9F 90 B7; 9F is an ISO control char.
        assert_eq!(
            message("file:///\u{f0}\u{9f}\u{90}\u{b7}"),
            "Illegal character in path at index 9: file:///\u{f0}\u{9f}\u{90}\u{b7}"
        );
        assert_eq!(
            message("file:///a\u{a0}b"),
            "Illegal character in path at index 9: file:///a\u{a0}b"
        );
        // Indices count chars, not UTF-8 bytes.
        assert_eq!(
            message("file:///\u{e9} x"),
            "Illegal character in path at index 9: file:///\u{e9} x"
        );
        assert_eq!(
            message("f\u{e9}:///x"),
            "Illegal character in scheme name at index 1: f\u{e9}:///x"
        );
    }

    #[test]
    fn escape_runs_decode_as_utf8_and_other_chars_are_copied() {
        assert_eq!(
            path_units("file:///a%C3%A9b\u{c3}\u{a9}"),
            [0x2f, 0x61, 0xe9, 0x62, 0xc3, 0xa9]
        );
        assert_eq!(path_units("file:///a%FFb"), [0x2f, 0x61, 0xfffd, 0x62]);
        // A run ending in a truncated sequence is replaced; the next raw char is kept.
        assert_eq!(path_units("file:///a%C3\u{a9}"), [0x2f, 0x61, 0xfffd, 0xa9]);
    }
}
