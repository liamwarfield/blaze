//! Port of `com.google.devtools.build.lib.shell.ShellUtils`.

use std::fmt;

/// Utility functions for Bourne shell commands, including escaping and tokenizing.
// Java: com/google/devtools/build/lib/shell/ShellUtils.java:23 ShellUtils
pub struct ShellUtils;

/// Characters that have no special meaning to the shell.
const SAFE_PUNCTUATION: &[u8] = b"@%-_+:,./";

/// `Character.isLetterOrDigit(c)`. The strings are Bazel's internal strings (chars up to
/// U+00FF, one per byte), for which Java's answer is written out; a char above U+00FF (a string
/// that is not internal) gets Rust's Unicode answer.
fn is_java_letter_or_digit(c: char) -> bool {
    match c as u32 {
        b @ 0..=0xFF => {
            let b = b as u8;
            b.is_ascii_alphanumeric()
                || b == 0xAA
                || b == 0xB5
                || b == 0xBA
                || (0xC0..=0xD6).contains(&b)
                || (0xD8..=0xF6).contains(&b)
                || b >= 0xF8
        }
        _ => c.is_alphanumeric(),
    }
}

/// Thrown by tokenize method if there is an error.
// Java: com/google/devtools/build/lib/shell/ShellUtils.java:74 ShellUtils.TokenizationException
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellUtilsTokenizationException {
    message: String,
}

impl ShellUtilsTokenizationException {
    fn new(message: &str) -> Self {
        ShellUtilsTokenizationException {
            message: message.to_string(),
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ShellUtilsTokenizationException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ShellUtilsTokenizationException {}

impl ShellUtils {
    /// Quotes a word so that it can be used, without further quoting, as an argument (or part of
    /// an argument) in a shell command.
    // Java: com/google/devtools/build/lib/shell/ShellUtils.java:36 ShellUtils.shellEscape
    pub fn shell_escape(word: &str) -> String {
        if word.is_empty() {
            // Empty string is a special case: needs to be quoted to ensure that it gets treated
            // as a separate argument.
            return "''".to_string();
        }
        for c in word.chars() {
            // We do this positively so as to be sure we don't inadvertently forget any unsafe
            // characters.
            if !is_java_letter_or_digit(c)
                && !(c.is_ascii() && SAFE_PUNCTUATION.contains(&(c as u8)))
            {
                // replace() actually means "replace all".
                return format!("'{}'", word.replace('\'', "'\\''"));
            }
        }
        word.to_string()
    }

    /// Given an argv array such as might be passed to execve(2), returns a string that can be
    /// copied and pasted into a Bourne shell for a similar effect.
    // Java: com/google/devtools/build/lib/shell/ShellUtils.java:59 ShellUtils.prettyPrintArgv
    pub fn pretty_print_argv<S: AsRef<str>>(argv: &[S]) -> String {
        let mut buf = String::new();
        for arg in argv {
            if !buf.is_empty() {
                buf.push(' ');
            }
            buf.push_str(&ShellUtils::shell_escape(arg.as_ref()));
        }
        buf
    }

    /// Populates the passed list of command-line options extracted from `option_string`, which
    /// is a string containing multiple options, delimited in a Bourne shell-like manner.
    ///
    /// Fails if there was an error (such as an unterminated quotation).
    // Java: com/google/devtools/build/lib/shell/ShellUtils.java:90 ShellUtils.tokenize
    pub fn tokenize(
        options: &mut Vec<String>,
        option_string: &str,
    ) -> Result<(), ShellUtilsTokenizationException> {
        // See test suite for examples.
        //
        // Note: backslash escapes the following character, except within a single-quoted region
        // where it is literal.
        let chars: Vec<char> = option_string.chars().collect();
        let len = chars.len();
        let mut token = String::new();
        let mut force_token = false;
        let mut quotation = '\0'; // NUL, '\'' or '"'
        let mut ii = 0;
        while ii < len {
            let mut c = chars[ii];
            if quotation != '\0' {
                // in quotation
                if c == quotation {
                    // end of quotation
                    quotation = '\0';
                } else if c == '\\' && quotation == '"' {
                    // backslash in "-quotation
                    ii += 1;
                    if ii == len {
                        return Err(ShellUtilsTokenizationException::new(
                            "backslash at end of string",
                        ));
                    }
                    c = chars[ii];
                    if c != '\\' && c != '"' {
                        token.push('\\');
                    }
                    token.push(c);
                } else {
                    // regular char, in quotation
                    token.push(c);
                }
            } else {
                // not in quotation
                if c == '\'' || c == '"' {
                    // begin single/double quotation
                    quotation = c;
                    force_token = true;
                } else if c == ' ' || c == '\t' {
                    // space, not quoted
                    if force_token || !token.is_empty() {
                        options.push(std::mem::take(&mut token));
                        force_token = false;
                    }
                } else if c == '\\' {
                    // backslash, not quoted
                    ii += 1;
                    if ii == len {
                        return Err(ShellUtilsTokenizationException::new(
                            "backslash at end of string",
                        ));
                    }
                    token.push(chars[ii]);
                } else {
                    // regular char, not quoted
                    token.push(c);
                }
            }
            ii += 1;
        }
        if quotation != '\0' {
            return Err(ShellUtilsTokenizationException::new(
                "unterminated quotation",
            ));
        }
        if force_token || !token.is_empty() {
            options.push(token);
        }
        Ok(())
    }

    /// Escape command line arguments for `CreateProcessW` on Windows.
    ///
    /// This method implements the same algorithm as the native xx_binary launcher does.
    // Java: com/google/devtools/build/lib/shell/ShellUtils.java:154 ShellUtils.windowsEscapeArg
    pub fn windows_escape_arg(s: &str) -> String {
        if s.is_empty() {
            return "\"\"".to_string();
        }
        let b = s.as_bytes();
        if !b.iter().any(|&c| c == b' ' || c == b'"') {
            return s.to_string();
        }

        // All special characters are ASCII, so byte offsets are char boundaries.
        let mut result = String::new();
        result.push('"');
        let mut start: isize = 0;
        let mut i = 0usize;
        while i < b.len() {
            let c = b[i];
            if c == b'"' || c == b'\\' {
                // Copy the segment since the last special character.
                if start >= 0 {
                    result.push_str(&s[start as usize..i]);
                    start = -1;
                }

                // Handle the current special character.
                if c == b'"' {
                    // This is a quote character. Escape it with a single backslash.
                    result.push_str("\\\"");
                } else {
                    // This is a backslash (or the first one in a run of backslashes).
                    // Whether we escape it depends on whether the run ends with a quote.
                    let mut run_len = 1;
                    let mut j = i + 1;
                    while j < b.len() && b[j] == b'\\' {
                        run_len += 1;
                        j += 1;
                    }
                    if j == b.len() {
                        // The run of backslashes goes to the end.
                        // We have to escape every backslash with another backslash.
                        for _ in 0..run_len * 2 {
                            result.push('\\');
                        }
                        break;
                    } else if b[j] == b'"' {
                        // The run of backslashes is terminated by a quote.
                        // We have to escape every backslash with another backslash, and
                        // escape the quote with one backslash.
                        for _ in 0..run_len * 2 {
                            result.push('\\');
                        }
                        result.push_str("\\\"");
                        i += run_len; // 'i' is also increased in the loop iteration step
                    } else {
                        // No quote found. Each backslash counts for itself, they must not be
                        // escaped.
                        for _ in 0..run_len {
                            result.push('\\');
                        }
                        i += run_len - 1; // 'i' is also increased in the loop iteration step
                    }
                }
            } else {
                // This is not a special character. Start the segment if necessary.
                if start < 0 {
                    start = i as isize;
                }
            }
            i += 1;
        }
        // Save final segment after the last special character.
        if start != -1 {
            result.push_str(&s[start as usize..]);
        }
        result.push('"');
        result
    }
}
