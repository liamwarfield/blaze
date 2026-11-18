//! The pattern matchers of `UnixGlob` that `IgnoredSubdirectories` and globbing use.
//!
//! PORT: move to blaze_vfs (Java `vfs/UnixGlob`). Not ported here: the glob builder and
//! traversal (Bazel 9 globs through Skyframe in GlobFunction instead).

/// How [`matches_pattern`] should work.
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:952 UnixGlob.MatchMode
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MatchMode {
    /// The path should exactly match the pattern.
    Exact,
    /// The pattern should match a prefix of the path.
    Prefix,
    /// Whether there can be any path under the prefix that matches the pattern.
    CanMatchChild,
}

/// Returns whether `pattern` matches `path`.
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:194 UnixGlob.matches
pub fn matches_segments(pattern: &[String], path: &[String]) -> bool {
    matches_pattern(pattern, path, 0, 0, MatchMode::Exact)
}

/// Returns whether any path under `path` can match `pattern`.
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:942 UnixGlob.canMatchChild
pub fn can_match_child(pattern: &[String], path: &[String]) -> bool {
    matches_pattern(pattern, path, 0, 0, MatchMode::CanMatchChild)
}

/// Returns whether `pattern` matches a prefix of `path`.
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:947 UnixGlob.matchesPrefix
pub fn matches_prefix(pattern: &[String], path: &[String]) -> bool {
    matches_pattern(pattern, path, 0, 0, MatchMode::Prefix)
}

/// Returns true if `pattern` matches `path` starting from the given segments.
///
/// PORT: Java's optional compiled-pattern cache is not ported (it only saves recompilation).
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:959 UnixGlob.matchesPattern
fn matches_pattern(
    pattern: &[String],
    path: &[String],
    i: usize,
    j: usize,
    match_mode: MatchMode,
) -> bool {
    if i == pattern.len() {
        return match_mode == MatchMode::Prefix || j == path.len();
    }
    if pattern[i] == "**" {
        return matches_pattern(pattern, path, i + 1, j, match_mode)
            || (j < path.len() && matches_pattern(pattern, path, i, j + 1, match_mode));
    }
    if j == path.len() {
        return match_mode == MatchMode::CanMatchChild;
    }
    if matches(&pattern[i], &path[j]) {
        return matches_pattern(pattern, path, i + 1, j + 1, match_mode);
    }
    false
}

/// Returns whether `s` matches the glob pattern `pattern`.
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:212 UnixGlob.matches
pub fn matches(pattern: &str, s: &str) -> bool {
    if pattern.is_empty() || s.is_empty() {
        return false;
    }

    // Common case: **
    if pattern == "**" {
        return true;
    }

    // Common case: *
    if pattern == "*" {
        return true;
    }

    // If a filename starts with '.', this char must be matched explicitly.
    if s.starts_with('.') && !pattern.starts_with('.') {
        return false;
    }

    // Common case: *.xyz
    if pattern.starts_with('*') && pattern.rfind('*') == Some(0) {
        return s.ends_with(&pattern[1..]);
    }
    // Common case: xyz*
    let last_index = pattern.len() - 1;
    // The first clause of this if statement is unnecessary, but is an
    // optimization--charAt runs faster than indexOf.
    if pattern.ends_with('*') && pattern.find('*') == Some(last_index) {
        return s.starts_with(&pattern[..last_index]);
    }

    let regex = make_pattern_from_wildcard(pattern);
    regex.matches(s)
}

/// One element of the regular expression Java compiles a wildcard into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Token {
    /// A literal character.
    Literal(char),
    /// `.`: any character except a line terminator.
    Any,
    /// `.*`.
    AnyMany,
}

/// The regular expression of a wildcard, as Java builds it: "*" becomes `.*`, "?" becomes `.`,
/// regexp metacharacters allowed in wildcards are escaped, and "(" / ")" are dropped.
///
/// PORT: Java compiles a `java.util.regex.Pattern`; the expression only ever contains literals,
/// `.` and `.*`, so Blaze matches it directly (`.` does not match line terminators, as in Java
/// without DOTALL).
struct WildcardPattern {
    tokens: Vec<Token>,
}

// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:259 UnixGlob.makePatternFromWildcard
fn make_pattern_from_wildcard(pattern: &str) -> WildcardPattern {
    let chars: Vec<char> = pattern.chars().collect();
    let len = chars.len();
    let mut regexp: Vec<Token> = Vec::new();
    let mut i = 0;
    while i < len {
        let c = chars[i];
        match c {
            '*' => {
                let mut to_increment = 0;
                if len > i + 1 && chars[i + 1] == '*' {
                    // The pattern '**' is interpreted to match 0 or more directory separators,
                    // not 1 or more. We skip the next * and then find a trailing/leading '/' and
                    // get rid of it.
                    to_increment = 1;
                    if len > i + 2 && chars[i + 2] == '/' {
                        // We have '**/' -- skip the '/'.
                        to_increment = 2;
                    } else if len == i + 2 && i > 0 && chars[i - 1] == '/' {
                        // We have '/**' -- remove the '/'.
                        regexp.pop();
                    }
                }
                regexp.push(Token::AnyMany);
                i += to_increment;
            }
            '?' => regexp.push(Token::Any),
            // The historical undocumented behavior of this function was to add '(' and ')' to the
            // regexp pattern string unescaped; Java now treats them as a safe no-op.
            '(' | ')' => {}
            // Escaped regexp special characters ('^', '$', '|', '+', '{', '}', '[', ']', '\\',
            // '.') and everything else match literally.
            _ => regexp.push(Token::Literal(c)),
        }
        i += 1;
    }
    WildcardPattern { tokens: regexp }
}

/// Java `Pattern.matcher(s).matches()` line terminators: `.` does not match these.
fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}')
}

impl WildcardPattern {
    fn matches(&self, s: &str) -> bool {
        let text: Vec<char> = s.chars().collect();
        // memo[t][p]: whether tokens[p..] match text[t..].
        let mut memo: Vec<Vec<Option<bool>>> =
            vec![vec![None; self.tokens.len() + 1]; text.len() + 1];
        self.match_from(&text, 0, 0, &mut memo)
    }

    fn match_from(
        &self,
        text: &[char],
        t: usize,
        p: usize,
        memo: &mut Vec<Vec<Option<bool>>>,
    ) -> bool {
        if let Some(r) = memo[t][p] {
            return r;
        }
        let r = if p == self.tokens.len() {
            t == text.len()
        } else {
            match self.tokens[p] {
                Token::Literal(c) => {
                    t < text.len() && text[t] == c && self.match_from(text, t + 1, p + 1, memo)
                }
                Token::Any => {
                    t < text.len()
                        && !is_line_terminator(text[t])
                        && self.match_from(text, t + 1, p + 1, memo)
                }
                Token::AnyMany => {
                    self.match_from(text, t, p + 1, memo)
                        || (t < text.len()
                            && !is_line_terminator(text[t])
                            && self.match_from(text, t + 1, p, memo))
                }
            }
        };
        memo[t][p] = Some(r);
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(p: &str) -> Vec<String> {
        p.split('/').map(String::from).collect()
    }

    #[test]
    fn wildcard_matching() {
        assert!(matches("*", "foo"));
        assert!(matches("*.cc", "foo.cc"));
        assert!(!matches("*.cc", "foo.h"));
        assert!(matches("foo*", "foobar"));
        assert!(matches("f?o", "foo"));
        assert!(!matches("f?o", "fooo"));
        assert!(matches("f*b*r", "foobar"));
        // Java: the "*" fast path matches before the leading-dot rule.
        assert!(matches("*", ".hidden"));
        assert!(!matches("*n", ".hidden"));
        assert!(matches(".h*", ".hidden"));
        assert!(matches("a(b)c", "abc"));
        assert!(matches("a.c", "a.c"));
        assert!(!matches("a.c", "abc"));
        assert!(!matches("", "a"));
    }

    #[test]
    fn segment_matching() {
        assert!(matches_prefix(&split("foo/**"), &split("foo/bar/baz")));
        assert!(matches_prefix(&split("foo"), &split("foo/bar")));
        assert!(!matches_prefix(&split("foo"), &split("fo/bar")));
        assert!(matches_segments(&split("**/BUILD"), &split("a/b/BUILD")));
        assert!(can_match_child(&split("a/b/c"), &split("a")));
        assert!(!can_match_child(&split("a/b/c"), &split("x")));
        assert!(can_match_child(&split("**/c"), &split("x")));
    }
}
