//! Ports of `lib/actions/ParameterFile.java`, `lib/util/GccParamFileEscaper.java` and
//! `lib/util/WindowsParamFileEscaper.java`.

use std::fmt;
use std::io::{self, Write};

use blaze_shell::ShellEscaper;
use blaze_vfs::PathFragment;

/// Different styles of parameter files.
// Java: com/google/devtools/build/lib/actions/ParameterFile.java:37 ParameterFile.ParameterFileType
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParameterFileType {
    /// A parameter file with every parameter on a separate line. This format cannot handle
    /// newlines in parameters. It is currently used for most tools, but may not be interpreted
    /// correctly if parameters contain white space or other special characters. It should be
    /// avoided for new development.
    UNQUOTED,

    /// A parameter file where each parameter is correctly quoted for shell use, and separated by
    /// white space (space, tab, newline). This format is safe for all characters, but must be
    /// specially supported by the tool. In particular, it must not be used with gcc and related
    /// tools, which do not support this format as it is.
    SHELL_QUOTED,

    /// A parameter file where each parameter is correctly quoted for gcc or clang use, and
    /// separated by white space (space, tab, newline).
    GCC_QUOTED,

    /// A parameter file where each parameter is correctly quoted for Windows use. Double-quotes
    /// are escaped, backslashes immediately preceding a double-quote are doubled, and empty
    /// parameters as well as parameters containing whitespace are surrounded in double-quotes.
    WINDOWS,
}

impl ParameterFileType {
    /// Java `Enum.name()` / `toString()`.
    pub fn name(self) -> &'static str {
        match self {
            ParameterFileType::UNQUOTED => "UNQUOTED",
            ParameterFileType::SHELL_QUOTED => "SHELL_QUOTED",
            ParameterFileType::GCC_QUOTED => "GCC_QUOTED",
            ParameterFileType::WINDOWS => "WINDOWS",
        }
    }

    /// Java `ParameterFileType.valueOf(name)`; `None` where Java throws
    /// `IllegalArgumentException`.
    pub fn value_of(name: &str) -> Option<ParameterFileType> {
        match name {
            "UNQUOTED" => Some(ParameterFileType::UNQUOTED),
            "SHELL_QUOTED" => Some(ParameterFileType::SHELL_QUOTED),
            "GCC_QUOTED" => Some(ParameterFileType::GCC_QUOTED),
            "WINDOWS" => Some(ParameterFileType::WINDOWS),
            _ => None,
        }
    }
}

// Java `Enum.toString()`.
impl fmt::Display for ParameterFileType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The extension of `ParameterFile.PARAMETER_FILE` (`FileType.of(".params")`).
// Java: com/google/devtools/build/lib/actions/ParameterFile.java:68 ParameterFile.PARAMETER_FILE
// PORT: `FileType` (lib/util) is not available to this crate; the extension it matches is kept.
pub const PARAMETER_FILE: &str = ".params";

/// Derives a path from a given path by appending `"-2.params"`.
// Java: com/google/devtools/build/lib/actions/ParameterFile.java:74 ParameterFile.derivePath
pub fn derive_path(original: &PathFragment) -> PathFragment {
    derive_path_with_flavor(original, "2")
}

/// Derives a path from a given path by appending `"-<flavor>.params"`.
// Java: com/google/devtools/build/lib/actions/ParameterFile.java:79 ParameterFile.derivePath
pub fn derive_path_with_flavor(original: &PathFragment, flavor: &str) -> PathFragment {
    let new_name = format!("{}-{}.params", original.get_base_name(), flavor);
    match original.replace_name(&new_name) {
        Some(p) => p,
        // Java: replaceName returns null for an empty fragment; the caller dereferences it.
        None => panic!("NullPointerException: replaceName of {:?}", original),
    }
}

/// Writes an argument list to a parameter file.
// Java: com/google/devtools/build/lib/actions/ParameterFile.java:84 ParameterFile.writeParameterFile
pub fn write_parameter_file<I, S>(
    out: &mut dyn Write,
    arguments: I,
    type_: ParameterFileType,
) -> io::Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    match type_ {
        ParameterFileType::SHELL_QUOTED => write_content(
            out,
            arguments
                .into_iter()
                .map(|a| ShellEscaper::escape_string(a.as_ref())),
        ),
        ParameterFileType::GCC_QUOTED => write_content(
            out,
            arguments
                .into_iter()
                .map(|a| GccParamFileEscaper::escape_string(a.as_ref())),
        ),
        ParameterFileType::UNQUOTED => write_content(out, arguments),
        ParameterFileType::WINDOWS => write_content(
            out,
            arguments
                .into_iter()
                .map(|a| WindowsParamFileEscaper::escape_string(a.as_ref())),
        ),
    }
}

// Java: com/google/devtools/build/lib/actions/ParameterFile.java:94 ParameterFile.writeContent
fn write_content<I, S>(out: &mut dyn Write, arguments: I) -> io::Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    for line in arguments {
        // Java: `out.write(StringUnsafe.getInternalStringBytes(line))`.
        out.write_all(&blaze_util::string_encoding::internal_string_bytes(
            line.as_ref(),
        ))?;
        out.write_all(b"\n")?;
    }
    out.flush()
}

/// Criterion shared by [`flags_only`] and [`non_flags`].
// Java: com/google/devtools/build/lib/actions/ParameterFile.java:104 ParameterFile.isFlag
fn is_flag(arg: &str) -> bool {
    arg.starts_with("--")
}

/// Filters the given args to only flags (i.e. start with "--").
///
/// Note, this makes sense only if flags with values have previously been joined, e.g.
/// "--foo=bar" rather than "--foo", "bar".
// Java: com/google/devtools/build/lib/actions/ParameterFile.java:114 ParameterFile.flagsOnly
pub fn flags_only(args: Vec<String>) -> Vec<String> {
    args.into_iter().filter(|a| is_flag(a)).collect()
}

/// Filters the given args to only non-flags (i.e. do not start with "--").
///
/// Note, this makes sense only if flags with values have previously been joined, e.g.
/// "--foo=bar" rather than "--foo", "bar".
// Java: com/google/devtools/build/lib/actions/ParameterFile.java:124 ParameterFile.nonFlags
pub fn non_flags(args: Vec<String>) -> Vec<String> {
    args.into_iter().filter(|a| !is_flag(a)).collect()
}

/// Utility class to escape strings for use in param files for gcc or clang.
///
/// Gcc and Clang interpret the following characters specially: single quote ('), double quote
/// ("), backslash (\), space ( ), tab (\t), carriage return (\r), newline (\n), form feed (\f),
/// and vertical tab (\u000B). All can be escaped by prefixing the symbol with a backslash.
// Java: com/google/devtools/build/lib/util/GccParamFileEscaper.java:32 GccParamFileEscaper
pub struct GccParamFileEscaper;

impl GccParamFileEscaper {
    // Java: com/google/devtools/build/lib/util/GccParamFileEscaper.java:37 GccParamFileEscaper.UNSAFECHAR_MATCHER
    fn is_unsafe_char(c: char) -> bool {
        matches!(
            c,
            '\'' | '"' | '\\' | ' ' | '\t' | '\r' | '\n' | '\u{000C}' | '\u{000B}'
        )
    }

    // Java: com/google/devtools/build/lib/util/GccParamFileEscaper.java:41 GccParamFileEscaper.escape
    pub fn escape(string: &str) -> String {
        if string.is_empty() {
            // Empty string is a special case: needs to be quoted to ensure that it
            // gets treated as a separate argument.
            return "''".to_string();
        }
        // Java: CharEscaper.escape with escape(char) below.
        let mut out = String::with_capacity(string.len());
        for c in string.chars() {
            // Java: com/google/devtools/build/lib/util/GccParamFileEscaper.java:53 GccParamFileEscaper.escape
            if Self::is_unsafe_char(c) {
                out.push('\\');
            }
            out.push(c);
        }
        out
    }

    // Java: com/google/devtools/build/lib/util/GccParamFileEscaper.java:64 GccParamFileEscaper.escapeString
    pub fn escape_string(unescaped: &str) -> String {
        GccParamFileEscaper::escape(unescaped)
    }

    /// Transforms the input of unescaped strings to escaped ones.
    // Java: com/google/devtools/build/lib/util/GccParamFileEscaper.java:72 GccParamFileEscaper.escapeAll
    pub fn escape_all<S: AsRef<str>>(unescaped: &[S]) -> Vec<String> {
        unescaped
            .iter()
            .map(|s| GccParamFileEscaper::escape_string(s.as_ref()))
            .collect()
    }
}

/// Utility class to escape strings for use in param files for windows lld-link.
// Java: com/google/devtools/build/lib/util/WindowsParamFileEscaper.java:21 WindowsParamFileEscaper
pub struct WindowsParamFileEscaper;

impl WindowsParamFileEscaper {
    /// Escapes the argument to be suitable for lld-link. Existing double-quotes are escaped, and
    /// empty arguments and arguments that contain whitespace are surrounded in unescaped
    /// double-quotes. Backslashes immediately before a double-quote are doubled.
    // Java: com/google/devtools/build/lib/util/WindowsParamFileEscaper.java:33 WindowsParamFileEscaper.escapeString
    pub fn escape_string(argument: &str) -> String {
        if !argument.is_empty() && !argument.contains('\\') && !argument.contains('"') {
            return if Self::contains_whitespace(argument) {
                format!("\"{}\"", argument)
            } else {
                argument.to_string()
            };
        }
        let needs_surrounding_quotes = argument.is_empty() || Self::contains_whitespace(argument);
        let mut out = String::with_capacity(argument.len() + 2);
        if needs_surrounding_quotes {
            out.push('"');
        }
        let mut backslashes = 0usize;
        for c in argument.chars() {
            if c == '\\' {
                backslashes += 1;
                continue;
            }
            if c == '"' {
                // Each literal backslash needs a pair, followed by one to escape the quote.
                out.push_str(&"\\".repeat(2 * backslashes + 1));
            } else {
                out.push_str(&"\\".repeat(backslashes));
            }
            out.push(c);
            backslashes = 0;
        }
        if needs_surrounding_quotes {
            // Keep trailing backslashes from escaping the closing quote.
            out.push_str(&"\\".repeat(2 * backslashes));
            out.push('"');
        } else {
            out.push_str(&"\\".repeat(backslashes));
        }
        out
    }

    // Java: com/google/devtools/build/lib/util/WindowsParamFileEscaper.java:71 WindowsParamFileEscaper.containsWhitespace
    fn contains_whitespace(argument: &str) -> bool {
        // Java: WHITESPACE_CHARACTERS = " ", "\t", "\n", "\r".
        [" ", "\t", "\n", "\r"].iter().any(|w| argument.contains(w))
    }

    /// Escapes each argument using [`WindowsParamFileEscaper::escape_string`].
    // Java: com/google/devtools/build/lib/util/WindowsParamFileEscaper.java:76 WindowsParamFileEscaper.escapeAll
    pub fn escape_all<S: AsRef<str>>(unescaped: &[S]) -> Vec<String> {
        unescaped
            .iter()
            .map(|s| WindowsParamFileEscaper::escape_string(s.as_ref()))
            .collect()
    }
}
