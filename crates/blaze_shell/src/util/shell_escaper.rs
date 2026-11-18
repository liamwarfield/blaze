//! Port of `com.google.devtools.build.lib.util.ShellEscaper`.
// PORT: move to blaze_util (it is a lib/util class; blaze_util does not have it yet).

/// Utility class to escape strings for use with shell commands.
///
/// Escaped strings may safely be inserted into shell commands. Escaping is only done if
/// necessary. Strings containing only shell-neutral characters will not be escaped. Unlike
/// `ShellUtils::shell_escape`, non-ASCII letters and digits are always escaped.
// Java: com/google/devtools/build/lib/util/ShellEscaper.java:45 ShellEscaper
pub struct ShellEscaper;

fn is_safe_char(c: char) -> bool {
    "@%-_+:,./".contains(c)
        || c.is_ascii_digit()
        || c.is_ascii_lowercase()
        || c.is_ascii_uppercase()
}

impl ShellEscaper {
    /// Escapes a string by adding strong (single) quotes around it if necessary.
    ///
    /// A string is not escaped iff it only contains safe characters (`[a-zA-Z0-9@%-_+:,./]`,
    /// plus `~` when it is not the first character). Escaped strings are created by replacing
    /// every occurrence of single quotes with the string `'\''` and enclosing the result in a
    /// pair of single quotes.
    // Java: com/google/devtools/build/lib/util/ShellEscaper.java:96 ShellEscaper.escape
    pub fn escape_string(unescaped: &str) -> String {
        let s = unescaped;
        if s.is_empty() {
            // Empty string is a special case: needs to be quoted to ensure that it gets treated
            // as a separate argument.
            return "''".to_string();
        }
        if s.chars().all(is_safe_char) {
            return s.to_string();
        }
        if s.chars().all(|c| is_safe_char(c) || c == '~') && !s.starts_with('~') {
            return s.to_string();
        }
        format!("'{}'", s.replace('\'', "'\\''"))
    }

    /// Transforms the input of unescaped strings to escaped ones.
    // Java: com/google/devtools/build/lib/util/ShellEscaper.java:121 ShellEscaper.escapeAll
    pub fn escape_all<S: AsRef<str>>(unescaped: &[S]) -> Vec<String> {
        unescaped
            .iter()
            .map(|s| ShellEscaper::escape_string(s.as_ref()))
            .collect()
    }

    /// Escapes all strings in `argv` individually and joins them on single spaces into `out`.
    // Java: com/google/devtools/build/lib/util/ShellEscaper.java:137 ShellEscaper.escapeJoinAll
    pub fn escape_join_all_into<'a, S: AsRef<str>>(
        out: &'a mut String,
        argv: &[S],
    ) -> &'a mut String {
        ShellEscaper::escape_join_all_into_with(out, argv, " ")
    }

    /// Escapes all strings in `argv` individually and joins them into `out` using `separator`
    /// (Java: a `Joiner`).
    // Java: com/google/devtools/build/lib/util/ShellEscaper.java:160 ShellEscaper.escapeJoinAll
    pub fn escape_join_all_into_with<'a, S: AsRef<str>>(
        out: &'a mut String,
        argv: &[S],
        separator: &str,
    ) -> &'a mut String {
        out.push_str(&ShellEscaper::escape_all(argv).join(separator));
        out
    }

    /// Escapes all strings in `argv` individually and joins them on single spaces.
    // Java: com/google/devtools/build/lib/util/ShellEscaper.java:178 ShellEscaper.escapeJoinAll
    pub fn escape_join_all<S: AsRef<str>>(argv: &[S]) -> String {
        ShellEscaper::escape_join_all_with(argv, " ")
    }

    /// Escapes all strings in `argv` individually and joins them using `separator`.
    // Java: com/google/devtools/build/lib/util/ShellEscaper.java:196 ShellEscaper.escapeJoinAll
    pub fn escape_join_all_with<S: AsRef<str>>(argv: &[S], separator: &str) -> String {
        ShellEscaper::escape_all(argv).join(separator)
    }
}
