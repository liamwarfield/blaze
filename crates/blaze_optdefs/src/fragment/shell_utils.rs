//! The part of `lib/shell/ShellUtils.java` that option converters use.
//!
//! PORT: ShellUtils belongs to the `blaze_shell` crate, which does not port it yet; the tokenizer
//! is ported here because `RunUnderConverter` needs it.

// Java: com/google/devtools/build/lib/shell/ShellUtils.java:74 ShellUtils.TokenizationException
/// Thrown by tokenize method if there is an error
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenizationException {
    message: String,
}

impl TokenizationException {
    pub fn get_message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for TokenizationException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

// Java: com/google/devtools/build/lib/shell/ShellUtils.java:90 ShellUtils.tokenize
/// Populates the passed list of command-line options extracted from `option_string`, which is a
/// string containing multiple options, delimited in a Bourne shell-like manner.
pub fn tokenize(
    options: &mut Vec<String>,
    option_string: &str,
) -> Result<(), TokenizationException> {
    // See test suite for examples.
    //
    // Note: backslash escapes the following character, except within a
    // single-quoted region where it is literal.
    let err = |m: &str| TokenizationException {
        message: m.to_string(),
    };
    // PORT: Java iterates UTF-16 chars; every special character is ASCII, so iterating code
    // points gives the same tokens.
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
                    return Err(err("backslash at end of string"));
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
                    return Err(err("backslash at end of string"));
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
        return Err(err("unterminated quotation"));
    }
    if force_token || !token.is_empty() {
        options.push(token);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(s: &str) -> Result<Vec<String>, TokenizationException> {
        let mut v = Vec::new();
        tokenize(&mut v, s).map(|_| v)
    }

    #[test]
    fn tokenize_basic() {
        assert_eq!(tok("a b\t c").unwrap(), vec!["a", "b", "c"]);
        assert_eq!(tok("'a b' \"c\\\"d\" ''").unwrap(), vec!["a b", "c\"d", ""]);
        assert_eq!(tok("a\\ b").unwrap(), vec!["a b"]);
        assert_eq!(tok("\"a\\b\"").unwrap(), vec!["a\\b"]);
        assert_eq!(
            tok("'abc").unwrap_err().get_message(),
            "unterminated quotation"
        );
        assert_eq!(
            tok("abc\\").unwrap_err().get_message(),
            "backslash at end of string"
        );
    }
}
