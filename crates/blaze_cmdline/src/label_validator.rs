//! Port of `LabelValidator`: the canonical place to validate package and target names.
//!
//! Names are internal strings (one char per byte of the real name) and are examined char by
//! char, as Java does.

/// Error for package names with characters outside the allowed set.
// Java: com/google/devtools/build/lib/cmdline/LabelValidator.java:79 LabelValidator.PACKAGE_NAME_ERROR
pub const PACKAGE_NAME_ERROR: &str = "package names may contain A-Z, a-z, 0-9, or any of ' !\"#$%&'()*+,-./;<=>?[]^_`{|}~' (any ASCII character except 0-31, 127, ':', or '\\')";

/// Error for package name segments made only of dots.
// Java: com/google/devtools/build/lib/cmdline/LabelValidator.java:84 LabelValidator.PACKAGE_NAME_DOT_ERROR
pub const PACKAGE_NAME_DOT_ERROR: &str = "package name component contains only '.' characters";

// Java: com/google/devtools/build/lib/cmdline/LabelValidator.java:39 LabelValidator.PUNCTUATION_REQUIRING_QUOTING
const PUNCTUATION_REQUIRING_QUOTING: &[u8] = b" \"#$&'()*+,;<=>?[]{|}~";

// Java: com/google/devtools/build/lib/cmdline/LabelValidator.java:48 LabelValidator.PUNCTUATION_NOT_REQUIRING_QUOTING
const PUNCTUATION_NOT_REQUIRING_QUOTING: &[u8] = b"!%-@^_`";

// Java: com/google/devtools/build/lib/cmdline/LabelValidator.java:56 LabelValidator.ALLOWED_CHARACTERS_IN_PACKAGE_NAME
fn allowed_character_in_package_name(c: u8) -> bool {
    c.is_ascii_digit()
        || c.is_ascii_lowercase()
        || c.is_ascii_uppercase()
        || b" !\"#$%&'()*+,-./;<=>?@[]^_`{|}~".contains(&c)
}

// Java: com/google/devtools/build/lib/cmdline/LabelValidator.java:69 LabelValidator.ALWAYS_ALLOWED_TARGET_CHARACTERS
fn always_allowed_target_character(c: char) -> bool {
    // CharMatcher.javaLetterOrDigit() restricted to ASCII; every non-ASCII char is accepted below.
    c.is_ascii_alphanumeric()
        || (c.is_ascii()
            && (PUNCTUATION_REQUIRING_QUOTING.contains(&(c as u8))
                || PUNCTUATION_NOT_REQUIRING_QUOTING.contains(&(c as u8))))
        // Accept any non-ASCII character, which is also guaranteed to be (part of) the encoding
        // of a non-ASCII code point, both for internal and Unicode strings.
        // PORT: `inRange(128, 65535)` on UTF-16 units; a Rust char above U+FFFF is a surrogate
        // pair in Java, both units in that range.
        || c as u32 >= 128
}

/// Validates a package name. Returns `None` on success or an error message.
// Java: com/google/devtools/build/lib/cmdline/LabelValidator.java:96 LabelValidator.validatePackageName
pub fn validate_package_name(package_name: &str) -> Option<String> {
    let b = package_name.as_bytes();
    let len = b.len();
    if len == 0 {
        // Empty package name (//:foo).
        return None;
    }

    if b[0] == b'/' {
        return Some("package names may not start with '/'".to_owned());
    }

    if !b.iter().all(|&c| allowed_character_in_package_name(c)) {
        return Some(PACKAGE_NAME_ERROR.to_owned());
    }

    // Every char is ASCII from here on, so the bytes are the chars.
    if b[len - 1] == b'/' {
        return Some("package names may not end with '/'".to_owned());
    }
    // Check for empty or dot-only package segment
    let mut non_dot = false;
    let mut last_slash = true;
    // Going backward and marking the last character as being a / so we detect
    // '.' only package segment.
    for i in (-1..len as isize).rev() {
        let c = if i >= 0 { b[i as usize] } else { b'/' };
        if c == b'/' {
            if last_slash {
                return Some("package names may not contain '//' path separators".to_owned());
            }
            if !non_dot {
                return Some(PACKAGE_NAME_DOT_ERROR.to_owned());
            }
            non_dot = false;
            last_slash = true;
        } else {
            if c != b'.' {
                non_dot = true;
            }
            last_slash = false;
        }
    }

    None // ok
}

/// Validates a target name. Returns `None` on success or an error message.
// Java: com/google/devtools/build/lib/cmdline/LabelValidator.java:146 LabelValidator.validateTargetName
pub fn validate_target_name(target_name: &str) -> Option<String> {
    // We allow labels equaling '.' or ending in '/.' for now. If we ever
    // actually configure the target we will report an error, but they are permitted for
    // data directories.

    // Code optimized for the common case: success.
    let Some(mut c) = target_name.chars().next() else {
        return Some("empty target name".to_owned());
    };
    // Forbidden start chars:
    if c == '/' {
        return Some("target names may not start with '/'".to_owned());
    } else if c == '.' {
        if target_name.starts_with("../") || target_name == ".." {
            return Some("target names may not contain up-level references '..'".to_owned());
        } else if target_name == "." {
            return None; // See comment above; ideally should be an error.
        } else if target_name.starts_with("./") {
            return Some("target names may not contain '.' as a path segment".to_owned());
        }
    }

    // Give a friendly error message on CRs in target names
    if target_name.ends_with('\r') {
        return Some(
            "target names may not end with carriage returns \
             (perhaps the input source is CRLF-terminated)"
                .to_owned(),
        );
    }

    // `ii` is a byte offset into the same string, used only for the region matches.
    for (ii, ch) in target_name.char_indices() {
        c = ch;
        if always_allowed_target_character(c) {
            continue;
        }
        if c == '.' {
            continue;
        }
        if c == '/' {
            if string_region_match(target_name, "/../", ii) {
                return Some("target names may not contain up-level references '..'".to_owned());
            } else if string_region_match(target_name, "/./", ii) {
                return Some("target names may not contain '.' as a path segment".to_owned());
            } else if string_region_match(target_name, "//", ii) {
                return Some("target names may not contain '//' path separators".to_owned());
            }
            continue;
        }
        if c <= '\u{1f}' || c == '\u{7f}' {
            return Some(format!(
                "target names may not contain non-printable characters: '\\x{:02X}'",
                c as u32
            ));
        }
        return Some(format!("target names may not contain '{}'", c));
    }
    // Forbidden end chars:
    if c == '.' {
        if target_name.ends_with("/..") {
            return Some("target names may not contain up-level references '..'".to_owned());
        } else if target_name.ends_with("/.") {
            return None; // See comment above; ideally should be an error.
        }
    }
    if c == '/' {
        return Some("target names may not end with '/'".to_owned());
    }
    None // ok
}

// Java: com/google/devtools/build/lib/cmdline/LabelValidator.java:216 LabelValidator.stringRegionMatch
fn string_region_match(full_string: &str, possible_match: &str, offset: usize) -> bool {
    full_string.as_bytes()[offset..].starts_with(possible_match.as_bytes())
}

/// Validates the label and parses it into package and target names.
///
/// Accepts `//foo/bar`, `//foo/bar:quux` and `//foo/bar:` (undocumented).
// Java: com/google/devtools/build/lib/cmdline/LabelValidator.java:231 LabelValidator.validateAbsoluteLabel
pub fn validate_absolute_label(
    abs_name: &str,
) -> Result<LabelValidatorPackageAndTarget, LabelValidatorBadLabelException> {
    let result = parse_absolute_label(abs_name)?;
    let package_name = result.get_package_name();
    let target_name = result.get_target_name();
    if let Some(error) = validate_package_name(package_name) {
        let mut error = format!("invalid package name '{}': {}", package_name, error);
        // This check is just for a more helpful error message,
        // i.e. valid target name, invalid package name, colon-free label form
        // used => probably they meant "//foo:bar.c" not "//foo/bar.c".
        if package_name.ends_with(&format!("/{}", target_name)) {
            error.push_str(&format!(" (perhaps you meant \":{}\"?)", target_name));
        }
        return Err(LabelValidatorBadLabelException::new(error));
    }
    if let Some(error) = validate_target_name(target_name) {
        return Err(LabelValidatorBadLabelException::new(format!(
            "invalid target name '{}': {}",
            target_name, error
        )));
    }
    Ok(result)
}

/// Returns if the label starts with a repository (@whatever) or a package (//whatever).
// Java: com/google/devtools/build/lib/cmdline/LabelValidator.java:257 LabelValidator.isAbsolute
pub fn is_absolute(label: &str) -> bool {
    label.starts_with("//") || label.starts_with('@')
}

/// Splits an absolute label into package and target names without validating them.
// Java: com/google/devtools/build/lib/cmdline/LabelValidator.java:272 LabelValidator.parseAbsoluteLabel
pub fn parse_absolute_label(
    abs_name: &str,
) -> Result<LabelValidatorPackageAndTarget, LabelValidatorBadLabelException> {
    if !is_absolute(abs_name) {
        return Err(LabelValidatorBadLabelException::new(format!(
            "invalid label: {}",
            abs_name
        )));
    }
    let mut abs_name = abs_name;
    if abs_name.starts_with('@') {
        match abs_name.find("//") {
            None => {
                return Ok(LabelValidatorPackageAndTarget::new("", &abs_name[1..]));
            }
            Some(end_of_repo) => abs_name = &abs_name[end_of_repo..],
        }
    }
    // Find the package/suffix separation:
    let split_at = abs_name.find(':').unwrap_or(abs_name.len());
    let package_name = &abs_name["//".len()..split_at];
    let suffix = &abs_name[split_at..];
    // ('suffix' is empty, or starts with a colon.)

    // "If packagename and version are elided, the colon is not necessary."
    let target_name = if suffix.is_empty() {
        // Target name is last package segment: (works in slash-free case too.)
        &package_name[package_name.rfind('/').map_or(0, |i| i + 1)..]
    } else {
        // Target name is what's after colon:
        &suffix[1..]
    };

    Ok(LabelValidatorPackageAndTarget::new(
        package_name,
        target_name,
    ))
}

/// A pair of package and target names, not necessarily valid (Java
/// `LabelValidator.PackageAndTarget`).
// Java: com/google/devtools/build/lib/cmdline/LabelValidator.java:304 LabelValidator.PackageAndTarget
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LabelValidatorPackageAndTarget {
    package_name: String,
    target_name: String,
}

impl LabelValidatorPackageAndTarget {
    // Java: com/google/devtools/build/lib/cmdline/LabelValidator.java:308 LabelValidator.PackageAndTarget.<init>
    pub fn new(package_name: &str, target_name: &str) -> Self {
        LabelValidatorPackageAndTarget {
            package_name: package_name.to_owned(),
            target_name: target_name.to_owned(),
        }
    }

    // Java: com/google/devtools/build/lib/cmdline/LabelValidator.java:313 LabelValidator.PackageAndTarget.getPackageName
    pub fn get_package_name(&self) -> &str {
        &self.package_name
    }

    // Java: com/google/devtools/build/lib/cmdline/LabelValidator.java:317 LabelValidator.PackageAndTarget.getTargetName
    pub fn get_target_name(&self) -> &str {
        &self.target_name
    }
}

// Java: com/google/devtools/build/lib/cmdline/LabelValidator.java:322 LabelValidator.PackageAndTarget.toString
impl std::fmt::Display for LabelValidatorPackageAndTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "//{}:{}", self.package_name, self.target_name)
    }
}

/// A label could not be parsed (Java `LabelValidator.BadLabelException`).
// Java: com/google/devtools/build/lib/cmdline/LabelValidator.java:345 LabelValidator.BadLabelException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct LabelValidatorBadLabelException {
    message: String,
}

impl LabelValidatorBadLabelException {
    pub fn new(message: impl Into<String>) -> Self {
        LabelValidatorBadLabelException {
            message: message.into(),
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }
}
