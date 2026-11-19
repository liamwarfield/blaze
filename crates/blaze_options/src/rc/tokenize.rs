//! Ports of the client string helpers used by rc parsing (`src/main/cpp/util/strings.cc`).

// Java: src/main/cpp/util/strings.cc:43 kSeparator
// The C++ tests a char with `strchr(kSeparator, c) != nullptr` (strings.cc:186, :235), which also
// matches the string's terminating NUL: a NUL byte separates tokens too.
const SEPARATOR: &[char] = &[' ', '\n', '\t', '\r', '\0'];

// Java: src/main/cpp/util/strings.cc:142 blaze_util::Replace
/// Replaces every occurrence of `oldsub` in `s` with `newsub`.
pub fn replace(oldsub: &str, newsub: &str, s: &mut String) {
    *s = s.replace(oldsub, newsub);
}

/// `ascii_isspace` from strings.h: TAB, LF, VT, FF, CR, space.
fn ascii_isspace(c: u8) -> bool {
    matches!(c, 9 | 10 | 11 | 12 | 13 | 32)
}

// Java: src/main/cpp/util/strings.cc:153 blaze_util::StripWhitespace
pub fn strip_whitespace(s: &mut String) {
    let b = s.as_bytes();
    let first = b.iter().position(|c| !ascii_isspace(*c));
    let Some(first) = first else {
        s.clear();
        return;
    };
    let last = b.iter().rposition(|c| !ascii_isspace(*c)).unwrap();
    *s = s[first..=last].to_string();
}

// Java: src/main/cpp/util/strings.cc:181 blaze_util::GetNextToken
// The C++ walks the bytes of a `std::string`; they are the chars of the internal string.
fn get_next_token(s: &[char], comment: char, iter: &mut usize, words: &mut Vec<String>) {
    let mut output = String::new();
    let mut last = *iter;
    let mut quote = '\0';
    while last < s.len() && (quote != '\0' || !SEPARATOR.contains(&s[last])) {
        // Absorb escapes.
        if s[last] == '\\' {
            last += 1;
            if last == s.len() {
                break;
            }
            output.push(s[last]);
            last += 1;
            continue;
        }
        if quote != '\0' {
            if s[last] == quote {
                // Absorb closing quote.
                quote = '\0';
                last += 1;
            } else {
                output.push(s[last]);
                last += 1;
            }
        } else {
            if s[last] == comment {
                last = s.len();
                break;
            }
            if s[last] == '\'' || s[last] == '"' {
                // Absorb opening quote.
                quote = s[last];
                last += 1;
            } else {
                output.push(s[last]);
                last += 1;
            }
        }
    }
    if !output.is_empty() {
        words.push(output);
    }
    *iter = last;
}

// Java: src/main/cpp/util/strings.cc:227 blaze_util::Tokenize
/// Splits `s` into words, treating `comment` as a comment start, honouring single and double
/// quotes and `\` escapes. Dangling escapes and missing end quotes are silently ignored.
pub fn tokenize(s: &str, comment: char) -> Vec<String> {
    let b: Vec<char> = s.chars().collect();
    let mut words = Vec::new();
    let mut i = 0;
    while i < b.len() {
        // Skip whitespace.
        while i < b.len() && SEPARATOR.contains(&b[i]) {
            i += 1;
        }
        if i < b.len() && b[i] == comment {
            break;
        }
        get_next_token(&b, comment, &mut i, &mut words);
    }
    words
}

// Java: src/main/cpp/util/strings.cc:85 blaze_util::SplitStringUsing
/// Splits on `delimiter`, dropping empty pieces at the ends and collapsing runs of delimiters.
pub fn split(contents: &str, delimiter: char) -> Vec<String> {
    contents
        .split(delimiter)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

// Java: src/main/cpp/util/path.cc:26 blaze_util::JoinPath
pub fn join_path(path1: &str, path2: &str) -> String {
    if path1.is_empty() {
        return path2.to_string();
    }
    if path1.ends_with('/') {
        if let Some(rest) = path2.strip_prefix('/') {
            format!("{}{}", path1, rest)
        } else {
            format!("{}{}", path1, path2)
        }
    } else if path2.starts_with('/') {
        format!("{}{}", path1, path2)
    } else {
        format!("{}/{}", path1, path2)
    }
}

#[cfg(test)]
mod byte_strings_tests {
    use super::*;

    // Byte-strings review 2026-11-20 S2-6. src/main/cpp/util/strings.cc:186,235 test separators
    // with `strchr(kSeparator, c)`, which is non-null for c == '\0' (the terminator matches), so
    // a NUL byte in an rc line splits tokens; inside quotes the C++ does not test separators.
    #[test]
    fn nul_separates_tokens() {
        assert_eq!(tokenize("build\0--foo", '#'), vec!["build", "--foo"]);
        assert_eq!(tokenize("\0build", '#'), vec!["build"]);
        assert_eq!(tokenize("'a\0b'", '#'), vec!["a\0b"]);
    }
}
