//! Port of `net.starlark.java.syntax.Location` as plain data.
//!
//! PORT: Blaze uses starlark-rust as its interpreter, so this is not the interpreter's location
//! type. It is the plain-data location Bazel's events carry and print; converters from
//! starlark-rust spans belong to the Starlark integration crate.

use std::cmp::Ordering;
use std::fmt;
use std::sync::LazyLock;

use blaze_util::check_argument;

/// Port of `net.starlark.java.syntax.Location`.
///
/// A location is a triple `(file, line, column)`, where `file` is the apparent name of the file,
/// `line` is the optional 1-based line number, and `column` is the optional 1-based column number
/// measured in UTF-16 code units. If the column is zero it is not displayed. If the line number is
/// also zero, it too is not displayed; in this case, the location denotes the file as a whole.
// Java: net/starlark/java/syntax/Location.java:30 Location
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Location {
    file: String,
    line: i32,
    column: i32,
}

impl Location {
    // Java: net/starlark/java/syntax/Location.java:36 Location.Location
    pub fn new(file: impl Into<String>, line: i32, column: i32) -> Self {
        Location {
            file: file.into(),
            line,
            column,
        }
    }

    /// Returns the name of the file containing this location.
    // Java: net/starlark/java/syntax/Location.java:43 Location.file
    pub fn file(&self) -> &str {
        &self.file
    }

    /// Returns the line number of this location.
    // Java: net/starlark/java/syntax/Location.java:48 Location.line
    pub fn line(&self) -> i32 {
        self.line
    }

    /// Returns the column number of this location.
    // Java: net/starlark/java/syntax/Location.java:53 Location.column
    pub fn column(&self) -> i32 {
        self.column
    }

    /// Returns a Location for the given file, line and column. If `column` is non-zero, `line` too
    /// must be non-zero.
    // Java: net/starlark/java/syntax/Location.java:61 Location.fromFileLineColumn
    pub fn from_file_line_column(file: impl Into<String>, line: i32, column: i32) -> Self {
        check_argument!(
            line != 0 || column == 0,
            "non-zero column but no line number"
        );
        Location::new(file, line, column)
    }

    /// Returns a Location for the file as a whole.
    // Java: net/starlark/java/syntax/Location.java:67 Location.fromFile
    pub fn from_file(file: impl Into<String>) -> Self {
        Location::new(file, 0, 0)
    }

    /// A location for built-in functions.
    // Java: net/starlark/java/syntax/Location.java:114 Location.BUILTIN
    pub fn builtin() -> &'static Location {
        static BUILTIN: LazyLock<Location> = LazyLock::new(|| Location::from_file("<builtin>"));
        &BUILTIN
    }
}

/// Formats the location as `"file:line:col"`. If the column is zero, it is omitted. If the line
/// is also zero, it too is omitted.
// Java: net/starlark/java/syntax/Location.java:76 Location.toString
impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.file)?;
        if self.line != 0 {
            write!(f, ":{}", self.line)?;
            if self.column != 0 {
                write!(f, ":{}", self.column)?;
            }
        }
        Ok(())
    }
}

/// Returns a three-valued lexicographical comparison of two Locations.
// Java: net/starlark/java/syntax/Location.java:90 Location.compareTo
impl Ord for Location {
    fn cmp(&self, that: &Self) -> Ordering {
        // PORT: Java String.compareTo compares UTF-16 code units; file names here are UTF-8
        // (StringEncoding identity), and byte order equals code-point order.
        let cmp = self.file.cmp(&that.file);
        if cmp != Ordering::Equal {
            return cmp;
        }
        // Java ORs a sign-extended int column into the shifted line; keep the same arithmetic.
        let a = ((self.line as i64) << 32) | (self.column as i64);
        let b = ((that.line as i64) << 32) | (that.column as i64);
        a.cmp(&b)
    }
}

impl PartialOrd for Location {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Java: src/test/java/net/starlark/java/syntax/LocationTest.java LocationTest.fromFile
    #[test]
    fn from_file() {
        let file = "this is a filename";
        let location = Location::from_file(file);
        assert_eq!(location.file(), file);
        assert_eq!(location.line(), 0);
        assert_eq!(location.column(), 0);
        assert_eq!(location.to_string(), file);
    }
}
