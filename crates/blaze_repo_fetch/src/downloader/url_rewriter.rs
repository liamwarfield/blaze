//! Ports of `UrlRewriter`, `UrlRewriterConfig` and `UrlRewriterParseException`
//! (`--downloader_config`).

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use base64::Engine as _;
use indexmap::{IndexMap, IndexSet};
use regex::Regex;

use blaze_events::location::Location;
use blaze_util::string_encoding::java_new_string_utf8;
use blaze_vfs::{Path, PathFragment};

use crate::authandtls::{Credentials, Headers, NetrcCredentials, netrc_parser};
use crate::uri::JavaUri;

/// An exception thrown when failing to parse the downloader config.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/UrlRewriterParseException.java:21 UrlRewriterParseException
#[derive(Clone, Debug, PartialEq)]
pub struct UrlRewriterParseException {
    pub message: String,
    pub location: Option<Location>,
}

impl UrlRewriterParseException {
    pub fn new(message: impl Into<String>, location: Option<Location>) -> Self {
        UrlRewriterParseException {
            message: message.into(),
            location,
        }
    }
}

impl fmt::Display for UrlRewriterParseException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for UrlRewriterParseException {}

/// A `rewrite` pattern: Java `Pattern` with `matches()` and `replaceFirst()`.
#[derive(Clone, Debug)]
pub struct RewritePattern {
    source: String,
    anchored: Regex,
    unanchored: Regex,
}

impl RewritePattern {
    /// PORT: Java `Pattern.compile`; Rust's `regex` syntax covers the patterns used in
    /// downloader configs (no backreferences or lookaround). A syntax error is reported with
    /// the Rust parser's description and index 0.
    pub fn compile(pattern: &str) -> Result<RewritePattern, (String, i32)> {
        let unanchored = Regex::new(pattern).map_err(|e| (e.to_string(), 0))?;
        let anchored = Regex::new(&format!("^(?:{})$", pattern)).map_err(|e| (e.to_string(), 0))?;
        Ok(RewritePattern {
            source: pattern.to_string(),
            anchored,
            unanchored,
        })
    }

    pub fn pattern(&self) -> &str {
        &self.source
    }

    /// `matcher(s).matches()`.
    pub fn matches(&self, s: &str) -> bool {
        self.anchored.is_match(s)
    }

    /// `matcher(s).replaceFirst(replacement)` with Java's replacement syntax (`$n`, `${name}`,
    /// `\x`).
    pub fn replace_first(&self, s: &str, replacement: &str) -> String {
        let Some(caps) = self.unanchored.captures(s) else {
            return s.to_string();
        };
        let whole = caps.get(0).unwrap();
        let group_count = self.unanchored.captures_len() - 1;
        let mut out = String::new();
        out.push_str(&s[..whole.start()]);
        let chars: Vec<char> = replacement.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '\\' {
                i += 1;
                if i >= chars.len() {
                    panic!("IllegalArgumentException: character to be escaped is missing");
                }
                out.push(chars[i]);
                i += 1;
            } else if c == '$' {
                i += 1;
                if i >= chars.len() {
                    panic!(
                        "IllegalArgumentException: Illegal group reference: group index is missing"
                    );
                }
                if chars[i] == '{' {
                    i += 1;
                    let start = i;
                    while i < chars.len() && chars[i] != '}' {
                        i += 1;
                    }
                    let name: String = chars[start..i].iter().collect();
                    i += 1;
                    match caps.name(&name) {
                        Some(m) => out.push_str(m.as_str()),
                        None => {
                            if self.unanchored.capture_names().flatten().all(|n| n != name) {
                                panic!("IllegalArgumentException: No group with name {{{}}}", name);
                            }
                        }
                    }
                } else {
                    let Some(d) = chars[i].to_digit(10) else {
                        panic!("IllegalArgumentException: Illegal group reference");
                    };
                    let mut ref_num = d as usize;
                    i += 1;
                    if ref_num > group_count {
                        panic!("IndexOutOfBoundsException: No group {}", ref_num);
                    }
                    // Capture the largest legal group string.
                    while i < chars.len() {
                        let Some(nd) = chars[i].to_digit(10) else {
                            break;
                        };
                        let new_ref = ref_num * 10 + nd as usize;
                        if new_ref <= group_count {
                            ref_num = new_ref;
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    if let Some(m) = caps.get(ref_num) {
                        out.push_str(m.as_str());
                    }
                }
            } else {
                out.push(c);
                i += 1;
            }
        }
        out.push_str(&s[whole.end()..]);
        out
    }
}

/// Models the downloader config file. Directives: `allow host`, `block host`,
/// `rewrite pattern replacement`, `all_blocked_message message`; `#` comments.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/UrlRewriterConfig.java:73 UrlRewriterConfig
#[derive(Clone, Debug)]
pub struct UrlRewriterConfig {
    allow_list: IndexSet<String>,
    block_list: IndexSet<String>,
    rewrites: Vec<(RewritePattern, Vec<String>)>,
    all_blocked_message: Option<String>,
}

const ALL_BLOCKED_MESSAGE_DIRECTIVE: &str = "all_blocked_message";

/// Guava `Splitter.onPattern("\\s+").omitEmptyStrings().trimResults()`.
fn split_whitespace_java(line: &str) -> Vec<&str> {
    line.split([' ', '\t', '\n', '\x0B', '\x0C', '\r'])
        .map(|s| s.trim_matches(is_guava_whitespace))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Guava `CharMatcher.whitespace()` (what `trimResults()` trims).
// Java: com/google/common/base/CharMatcher.java Whitespace.TABLE (Guava)
fn is_guava_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\u{000B}'
            | '\u{000C}'
            | '\r'
            | ' '
            | '\u{0085}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

impl UrlRewriterConfig {
    /// Constructor for one file; the content is read to completion.
    pub fn new(
        file_path_for_error_reporting: &str,
        config: &str,
    ) -> Result<Self, UrlRewriterParseException> {
        Self::new_multi(
            &[file_path_for_error_reporting.to_string()],
            &[config.to_string()],
        )
    }

    /// Constructor for multiple files.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/UrlRewriterConfig.java:110 UrlRewriterConfig.<init>
    pub fn new_multi(
        file_paths_for_error_reporting: &[String],
        configs: &[String],
    ) -> Result<Self, UrlRewriterParseException> {
        assert_eq!(
            file_paths_for_error_reporting.len(),
            configs.len(),
            "Number of file paths do not match number of readers"
        );
        let mut cfg = UrlRewriterConfig {
            allow_list: IndexSet::new(),
            block_list: IndexSet::new(),
            rewrites: Vec::new(),
            all_blocked_message: None,
        };
        let mut all_blocked_message = String::new();
        for (path, config) in file_paths_for_error_reporting.iter().zip(configs) {
            cfg.parse_config(config, path, &mut all_blocked_message)?;
        }
        if !all_blocked_message.is_empty() {
            cfg.all_blocked_message = Some(all_blocked_message);
        }
        Ok(cfg)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/UrlRewriterConfig.java:136 UrlRewriterConfig.parseConfig
    fn parse_config(
        &mut self,
        config: &str,
        file_path_for_error_reporting: &str,
        all_blocked_message: &mut String,
    ) -> Result<(), UrlRewriterParseException> {
        for (idx, line) in crate::fs_utils::split_lines(config).iter().enumerate() {
            let line_number = (idx + 1) as i32;
            // Find the first word
            let parts = split_whitespace_java(line);
            if parts.is_empty() {
                continue;
            }
            // Allow comments to use #
            if parts[0].starts_with('#') {
                continue;
            }
            let location = Some(Location::from_file_line_column(
                file_path_for_error_reporting,
                line_number,
                0,
            ));
            match parts[0] {
                "allow" => {
                    if parts.len() != 2 {
                        return Err(UrlRewriterParseException::new(
                            format!("Only the host name is allowed after `allow`: {}", line),
                            location,
                        ));
                    }
                    self.allow_list.insert(parts[1].to_string());
                }
                "block" => {
                    if parts.len() != 2 {
                        return Err(UrlRewriterParseException::new(
                            format!("Only the host name is allowed after `block`: {}", line),
                            location,
                        ));
                    }
                    self.block_list.insert(parts[1].to_string());
                }
                "rewrite" => {
                    if parts.len() != 3 {
                        return Err(UrlRewriterParseException::new(
                            format!(
                                "Only the matching pattern and rewrite pattern is allowed after `rewrite`: {}",
                                line
                            ),
                            location,
                        ));
                    }
                    match RewritePattern::compile(parts[1]) {
                        Ok(p) => self.rewrites.push((p, vec![parts[2].to_string()])),
                        Err((description, index)) => {
                            return Err(UrlRewriterParseException::new(
                                format!(
                                    "Invalid regex in `rewrite`: {} at index {} in `{}`",
                                    description, index, parts[1]
                                ),
                                location,
                            ));
                        }
                    }
                }
                ALL_BLOCKED_MESSAGE_DIRECTIVE => {
                    if parts.len() == 1 {
                        return Err(UrlRewriterParseException::new(
                            "all_blocked_message must be followed by a message",
                            location,
                        ));
                    }
                    if !all_blocked_message.is_empty() {
                        return Err(UrlRewriterParseException::new(
                            "At most one all_blocked_message directive is allowed",
                            location,
                        ));
                    }
                    all_blocked_message.push_str(&line[ALL_BLOCKED_MESSAGE_DIRECTIVE.len() + 1..]);
                }
                _ => {
                    return Err(UrlRewriterParseException::new(
                        format!("Unable to parse: {}", line),
                        location,
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn get_allow_list(&self) -> &IndexSet<String> {
        &self.allow_list
    }

    pub fn get_block_list(&self) -> &IndexSet<String> {
        &self.block_list
    }

    /// Patterns (matching everything in the URL after the scheme) with their replacements.
    pub fn get_rewrites(&self) -> &[(RewritePattern, Vec<String>)] {
        &self.rewrites
    }

    pub fn get_all_blocked_message(&self) -> Option<&str> {
        self.all_blocked_message.as_deref()
    }
}

/// Holds the URL along with meta-info, such as whether URL was re-written or not.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/UrlRewriter.java:350 UrlRewriter.RewrittenURL
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RewrittenUrl {
    pub url: JavaUri,
    pub rewritten: bool,
}

const REWRITABLE_SCHEMES: [&str; 2] = ["http", "https"];

/// Rewrites URLs according to the `--downloader_config` file.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/UrlRewriter.java:52 UrlRewriter
#[derive(Clone, Debug)]
pub struct UrlRewriter {
    config: UrlRewriterConfig,
}

impl UrlRewriter {
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/UrlRewriter.java:58 UrlRewriter.<init>
    pub fn new(
        file_paths_for_error_reporting: &[String],
        contents: &[String],
    ) -> Result<UrlRewriter, UrlRewriterParseException> {
        assert_eq!(
            file_paths_for_error_reporting.len(),
            contents.len(),
            "filePath and readers size must be equal"
        );
        Ok(UrlRewriter {
            config: UrlRewriterConfig::new_multi(file_paths_for_error_reporting, contents)?,
        })
    }

    /// Obtains a `UrlRewriter` configured with the specified config file (may be absent).
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/UrlRewriter.java:76 UrlRewriter.getDownloaderUrlRewriter
    pub fn get_downloader_url_rewriter(
        workspace_root: &Path,
        config_path: Option<&PathFragment>,
    ) -> Result<UrlRewriter, UrlRewriterParseException> {
        // "empty" UrlRewriter shouldn't alter auth headers
        let Some(config_path) = config_path.filter(|p| !p.is_empty()) else {
            return UrlRewriter::new(&[String::new()], &[String::new()]);
        };
        let actual_config_path = workspace_root.get_relative(config_path);
        if !actual_config_path.exists() {
            return Err(UrlRewriterParseException::new(
                format!(
                    "Unable to find downloader config file {}",
                    actual_config_path.get_path_string()
                ),
                None,
            ));
        }
        let content = crate::fs_utils::read_content(&actual_config_path).map_err(|e| {
            UrlRewriterParseException::new(crate::errors::IoException::from(e).get_message(), None)
        })?;
        UrlRewriter::new(
            &[config_path.get_path_string().to_string()],
            // Java: new InputStreamReader(actualConfigPath.getInputStream(), UTF_8): Unicode text.
            &[java_new_string_utf8(&content)],
        )
    }

    /// Rewrites `urls`; the result may be empty if the configuration blocks all of them.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/UrlRewriter.java:145 UrlRewriter.amend
    pub fn amend(&self, urls: &[JavaUri]) -> Vec<RewrittenUrl> {
        urls.iter().flat_map(|u| self.rewrite(u)).collect()
    }

    /// Updates `auth_headers` using the userInfo of rewritten URLs, else the netrc credentials.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/UrlRewriter.java:160 UrlRewriter.updateAuthHeaders
    pub fn update_auth_headers(
        &self,
        urls: &[RewrittenUrl],
        auth_headers: &HashMap<JavaUri, Headers>,
        netrc_creds: Option<&dyn Credentials>,
    ) -> HashMap<JavaUri, Headers> {
        let mut updated = auth_headers.clone();
        for url in urls {
            // if URL was not re-written by UrlRewriter in first place, we should not attach
            // auth headers to it
            if !url.rewritten {
                continue;
            }
            if let Some(user_info) = url.url.get_user_info() {
                // Java `userInfo.getBytes(ISO_8859_1)`: `?` for a char above U+00FF.
                let token = format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD
                        .encode(blaze_util::string_encoding::latin1_encode_lossy(&user_info))
                );
                let mut h = IndexMap::new();
                h.insert("Authorization".to_string(), vec![token]);
                updated.insert(url.url.clone(), h);
            } else if let Some(creds) = netrc_creds {
                // If the credentials extraction failed, we're letting bazel try without them.
                let Ok(url_auth_headers) = creds.get_request_metadata(&url.url) else {
                    continue;
                };
                // there could be multiple Auth headers, take the first one
                if let Some((k, v)) = url_auth_headers.first()
                    && !v.is_empty()
                {
                    let mut h = IndexMap::new();
                    h.insert(k.clone(), vec![v[0].clone()]);
                    updated.insert(url.url.clone(), h);
                }
            }
        }
        updated
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/UrlRewriter.java:200 UrlRewriter.rewrite
    fn rewrite(&self, url: &JavaUri) -> Vec<RewrittenUrl> {
        // Cowardly refuse to rewrite non-HTTP(S) urls
        if !REWRITABLE_SCHEMES.iter().any(|s| {
            url.get_scheme()
                .is_some_and(|us| us.eq_ignore_ascii_case(s))
        }) {
            return vec![RewrittenUrl {
                url: url.clone(),
                rewritten: false,
            }];
        }
        let mut to_return = Vec::new();
        for consider in self.apply_rewrite_rules(url) {
            // If there's an allow entry, add it to the set to return and continue
            if self.is_allow_matched(&consider.url) {
                to_return.push(consider);
                continue;
            }
            // If there's no block that matches the domain, add it to the set to return
            if !self.is_block_matched(&consider.url) {
                to_return.push(consider);
            }
        }
        to_return
    }

    fn is_allow_matched(&self, url: &JavaUri) -> bool {
        self.config
            .get_allow_list()
            .iter()
            .any(|h| is_matching_host_name(url, h))
    }

    fn is_block_matched(&self, url: &JavaUri) -> bool {
        self.config
            .get_block_list()
            .iter()
            .any(|h| h == "*" || is_matching_host_name(url, h))
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/UrlRewriter.java:254 UrlRewriter.applyRewriteRules
    fn apply_rewrite_rules(&self, url: &JavaUri) -> Vec<RewrittenUrl> {
        let scheme = url.get_scheme().unwrap_or("");
        let without_scheme = url.as_str().get(scheme.len() + 3..).unwrap_or("");
        let mut rewritten_urls: IndexSet<String> = IndexSet::new();
        let mut match_made = false;
        for (pattern, replacements) in self.config.get_rewrites() {
            if pattern.matches(without_scheme) {
                match_made = true;
                for replacement in replacements {
                    rewritten_urls.insert(pattern.replace_first(without_scheme, replacement));
                }
            }
        }
        if !match_made {
            return vec![RewrittenUrl {
                url: url.clone(),
                rewritten: false,
            }];
        }
        rewritten_urls
            .into_iter()
            .map(|s| RewrittenUrl {
                url: prefix_with_protocol(&s, scheme),
                rewritten: true,
            })
            .collect()
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/UrlRewriter.java:339 UrlRewriter.getAllBlockedMessage
    pub fn get_all_blocked_message(&self) -> Option<&str> {
        self.config.get_all_blocked_message()
    }

    /// Creates [`Credentials`] from `$NETRC`, else `$HOME/.netrc`; `None` if there is none.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/UrlRewriter.java:301 UrlRewriter.newCredentialsFromNetrc
    pub fn new_credentials_from_netrc(
        client_env: &HashMap<String, String>,
        working_directory: Option<&Path>,
    ) -> Result<Option<Arc<dyn Credentials>>, UrlRewriterParseException> {
        let home_dir = client_env.get("HOME");
        let netrc_file_string = match client_env.get("NETRC") {
            Some(n) => n.clone(),
            None => match home_dir {
                Some(h) => format!("{}/.netrc", h),
                None => return Ok(None),
            },
        };
        let location = Location::from_file_line_column(netrc_file_string.clone(), 0, 0);
        // In case Bazel is not started from a valid workspace.
        let Some(working_directory) = working_directory else {
            return Ok(None);
        };
        let netrc_file = working_directory.get_relative_str(&netrc_file_string);
        if !netrc_file.exists() {
            return Ok(None);
        }
        let parsed = netrc_file
            .get_input_stream()
            .map_err(crate::errors::IoException::from)
            .and_then(netrc_parser::parse_and_close);
        match parsed {
            Ok(netrc) => Ok(Some(Arc::new(NetrcCredentials::new(netrc)))),
            Err(e) => Err(UrlRewriterParseException::new(
                format!(
                    "Failed to parse {}: {}",
                    netrc_file.get_path_string(),
                    e.get_message()
                ),
                Some(location),
            )),
        }
    }
}

fn is_matching_host_name(url: &JavaUri, host: &str) -> bool {
    url.get_host()
        .is_some_and(|url_host| host == url_host || url_host.ends_with(&format!(".{}", host)))
}

/// Prefixes url with protocol if not already prefixed by a rewritable scheme.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/UrlRewriter.java:285 UrlRewriter.prefixWithProtocol
fn prefix_with_protocol(url: &str, protocol: &str) -> JavaUri {
    let s = if REWRITABLE_SCHEMES
        .iter()
        .any(|p| url.starts_with(&format!("{}://", p)))
    {
        url.to_string()
    } else {
        format!("{}://{}", protocol, url)
    };
    // URI.create throws IllegalArgumentException (a crash) on malformed rewrites.
    JavaUri::create(&s).unwrap_or_else(|e| panic!("IllegalArgumentException: {}", e))
}
