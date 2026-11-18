//! Port of `lib/actions/SingleStringArgFormatter.java`.
//!
//! Implementation of a formatter that supports only a single '%s'.
//!
//! This implementation is used in command line item expansions that use formatting. We use a
//! custom implementation to improve performance and avoid GC.
//!
//! PORT: Java indexes UTF-16 chars of an internal (Latin-1) string, i.e. bytes; the port indexes
//! the UTF-8 bytes. '%' and 's' are ASCII, so slicing at their positions is always valid.

/// Returns true if the format string is a valid single-arg formatter.
///
/// Requirements are:
///
/// * Contains exactly one '%s'.
/// * Each occurrence of '%' is either '%s' or '%%' (escape sequence).
// Java: com/google/devtools/build/lib/actions/SingleStringArgFormatter.java:36 SingleStringArgFormatter.isValid
pub fn is_valid(format_str: &str) -> bool {
    formatted_length_or_invalid(format_str) != -1
}

/// Calculates the format specifier's contribution to the length of a string created by calling
/// [`format`], without actually applying any formatting.
///
/// For a typical format specifier with no escape characters, returns `format_str.len() - 2`,
/// since the `%s` gets replaced during formatting. The result may differ if the format specifier
/// contains escape characters.
///
/// Panics (Java `IllegalArgumentException`) if the format string is invalid.
// Java: com/google/devtools/build/lib/actions/SingleStringArgFormatter.java:56 SingleStringArgFormatter.formattedLength
pub fn formatted_length(format_str: &str) -> i32 {
    let length = formatted_length_or_invalid(format_str);
    if length == -1 {
        invalid_format_string(format_str);
    }
    length
}

/// Returns the formatted length or `-1` if invalid.
// Java: com/google/devtools/build/lib/actions/SingleStringArgFormatter.java:65 SingleStringArgFormatter.formattedLengthOrInvalid
fn formatted_length_or_invalid(format_str: &str) -> i32 {
    let bytes = format_str.as_bytes();
    let mut length: i32 = 0;
    let n = bytes.len();
    let mut idx = 0usize;
    let mut found = false;

    while idx < n {
        let next = match index_of_percent(bytes, idx) {
            None => {
                length += (n - idx) as i32;
                break;
            }
            Some(next) => next,
        };
        if next == n - 1 {
            return -1; // Terminating '%'.
        }
        match bytes[next + 1] {
            b's' => {
                if found {
                    return -1; // Multiple '%s'.
                }
                length += (next - idx) as i32;
                found = true;
            }
            b'%' => length += (next + 1 - idx) as i32,
            _ => {
                return -1; // Illegal sequence.
            }
        }
        idx = next + 2;
    }

    if found { length } else { -1 }
}

/// Returns the equivalent result of `String.format(formatStr, subject)`, under the assumption
/// that the format string contains a single %s.
///
/// Use [`is_valid`] to validate the format string.
///
/// Panics (Java `IllegalArgumentException`) if the format string is invalid.
// Java: com/google/devtools/build/lib/actions/SingleStringArgFormatter.java:107 SingleStringArgFormatter.format
pub fn format(format_str: &str, subject: &str) -> String {
    let bytes = format_str.as_bytes();
    let mut sb = String::with_capacity((format_str.len() + subject.len()).saturating_sub(2));
    let n = bytes.len();
    let mut idx = 0usize;
    let mut found = false;

    while idx < n {
        let next = match index_of_percent(bytes, idx) {
            None => {
                sb.push_str(&format_str[idx..n]);
                break;
            }
            Some(next) => next,
        };
        if next == n - 1 {
            invalid_format_string(format_str); // Terminating '%'.
        }
        match bytes[next + 1] {
            b's' => {
                if found {
                    invalid_format_string(format_str); // Multiple '%s'.
                }
                sb.push_str(&format_str[idx..next]);
                sb.push_str(subject);
                found = true;
            }
            b'%' => sb.push_str(&format_str[idx..next + 1]),
            _ => invalid_format_string(format_str), // Illegal sequence.
        }
        idx = next + 2;
    }

    if !found {
        invalid_format_string(format_str); // No '%s'.
    }
    sb
}

/// Java `formatStr.indexOf('%', idx)`.
fn index_of_percent(bytes: &[u8], from: usize) -> Option<usize> {
    bytes[from..]
        .iter()
        .position(|&b| b == b'%')
        .map(|i| i + from)
}

// Java: com/google/devtools/build/lib/actions/SingleStringArgFormatter.java:142 SingleStringArgFormatter.invalidFormatString
fn invalid_format_string(format_str: &str) -> ! {
    panic!(
        "IllegalArgumentException: Expected format string with single '%s', found: {}",
        format_str
    )
}
