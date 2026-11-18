//! Port of `LabelParser`: utilities to help parse labels.

use crate::label_syntax_exception::LabelSyntaxException;
use crate::label_validator;
use crate::repository_name::RepositoryName;
use crate::string_utilities::sanitize_control_chars;

/// The parsed, validated elements of a label string (Java `LabelParser.Parts`).
///
/// ```text
///  raw                  | repo   | repoIs-   | pkgIs-   | pkg       | pkgEndsWith- | target
///                       |        | Canonical | Absolute |           | TripleDots   |
/// ----------------------+--------+-----------+----------+-----------+--------------+-----------
/// "foo/bar"             | null   | false     | false    | ""        | false        | "foo/bar"
/// "..."                 | null   | false     | false    | ""        | true         | ""
/// "...:all"             | null   | false     | false    | ""        | true         | "all"
/// "foo/..."             | null   | false     | false    | "foo"     | true         | ""
/// "//foo/bar"           | null   | false     | true     | "foo/bar" | false        | "bar"
/// "//foo/..."           | null   | false     | true     | "foo"     | true         | ""
/// "//foo/...:all"       | null   | false     | true     | "foo"     | true         | "all"
/// "//foo/all"           | null   | false     | true     | "foo/all" | false        | "all"
/// "@repo"               | "repo" | false     | true     | ""        | false        | "repo"
/// "@@repo"              | "repo" | true      | true     | ""        | false        | "repo"
/// "@repo//foo/bar"      | "repo" | false     | true     | "foo/bar" | false        | "bar"
/// "@@repo//foo/bar"     | "repo" | true      | true     | "foo/bar" | false        | "bar"
/// ":quux"               | null   | false     | false    | ""        | false        | "quux"
/// "foo/bar:quux"        | null   | false     | false    | "foo/bar" | false        | "quux"
/// "//foo/bar:quux"      | null   | false     | true     | "foo/bar" | false        | "quux"
/// "@repo//foo/bar:quux" | "repo" | false     | true     | "foo/bar" | false        | "quux"
/// ```
// Java: com/google/devtools/build/lib/cmdline/LabelParser.java:32 LabelParser.Parts
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LabelParserParts {
    repo: Option<String>,
    repo_is_canonical: bool,
    pkg_is_absolute: bool,
    pkg: String,
    pkg_ends_with_triple_dots: bool,
    target: String,
    raw: String,
}

impl LabelParserParts {
    /// The `@repo` or `@@canonical_repo` part (sans leading "@"s), if any.
    pub fn repo(&self) -> Option<&str> {
        self.repo.as_deref()
    }

    /// Whether the repo part uses the canonical repo syntax (two "@"s).
    pub fn repo_is_canonical(&self) -> bool {
        self.repo_is_canonical
    }

    /// Whether the package part is prefixed by double-slash.
    pub fn pkg_is_absolute(&self) -> bool {
        self.pkg_is_absolute
    }

    /// The package part (sans leading "//" and sans a final "..." segment).
    pub fn pkg(&self) -> &str {
        &self.pkg
    }

    /// Whether the package part ends with a "..." segment.
    pub fn pkg_ends_with_triple_dots(&self) -> bool {
        self.pkg_ends_with_triple_dots
    }

    /// The target part (sans colon).
    pub fn target(&self) -> &str {
        &self.target
    }

    /// The original unparsed raw string.
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Validates the parts and creates the value. Visible for testing.
    // Java: com/google/devtools/build/lib/cmdline/LabelParser.java:63 LabelParser.Parts.validateAndCreate
    pub fn validate_and_create(
        repo: Option<&str>,
        repo_is_canonical: bool,
        pkg_is_absolute: bool,
        pkg: &str,
        pkg_ends_with_triple_dots: bool,
        target: &str,
        raw: &str,
    ) -> Result<LabelParserParts, LabelSyntaxException> {
        validate_repo_name(repo)?;
        validate_package_name(pkg, target)?;
        Ok(LabelParserParts {
            repo: repo.map(str::to_owned),
            repo_is_canonical,
            pkg_is_absolute,
            pkg: pkg.to_owned(),
            pkg_ends_with_triple_dots,
            target: validate_and_process_target_name(pkg, target, pkg_ends_with_triple_dots)?,
            raw: raw.to_owned(),
        })
    }

    /// Parses a raw label string into parts (see the table on [`LabelParserParts`]).
    // Java: com/google/devtools/build/lib/cmdline/LabelParser.java:109 LabelParser.Parts.parse
    pub fn parse(raw_label: &str) -> Result<LabelParserParts, LabelSyntaxException> {
        let repo: Option<&str>;
        let repo_is_canonical = raw_label.starts_with("@@");
        let start_of_package: usize;
        let double_slash_index = raw_label.find("//");
        let pkg_is_absolute: bool;
        if raw_label.starts_with('@') {
            let repo_start = if repo_is_canonical { 2 } else { 1 };
            match double_slash_index {
                None => {
                    // Special case: the label "@foo" is synonymous with "@foo//:foo".
                    let repo = &raw_label[repo_start..];
                    return Self::validate_and_create(
                        Some(repo),
                        repo_is_canonical,
                        /* pkg_is_absolute= */ true,
                        /* pkg= */ "",
                        /* pkg_ends_with_triple_dots= */ false,
                        /* target= */ repo,
                        raw_label,
                    );
                }
                Some(double_slash_index) => {
                    repo = Some(&raw_label[repo_start..double_slash_index]);
                    start_of_package = double_slash_index + 2;
                    pkg_is_absolute = true;
                }
            }
        } else {
            // If the label begins with '//', it's an absolute label. Otherwise, treat it as
            // relative (the command-line kind).
            pkg_is_absolute = double_slash_index == Some(0);
            start_of_package = if double_slash_index == Some(0) { 2 } else { 0 };
            repo = None;
        }

        let pkg: String;
        let target: String;
        let colon_index = raw_label[start_of_package..]
            .find(':')
            .map(|i| i + start_of_package);
        let raw_pkg = &raw_label[start_of_package..colon_index.unwrap_or(raw_label.len())];
        let pkg_ends_with_triple_dots = raw_pkg.ends_with("/...") || raw_pkg == "...";
        if colon_index.is_none() && pkg_ends_with_triple_dots {
            // Special case: if the entire label ends in '...', the target name is empty.
            pkg = strip_trailing_triple_dots(raw_pkg).to_owned();
            target = String::new();
        } else if colon_index.is_none() && !pkg_is_absolute {
            // Special case: the label "foo/bar" is synonymous with ":foo/bar".
            pkg = String::new();
            target = raw_label[start_of_package..].to_owned();
        } else {
            pkg = strip_trailing_triple_dots(raw_pkg).to_owned();
            target = match colon_index {
                Some(colon_index) => raw_label[colon_index + 1..].to_owned(),
                None => {
                    // Special case: the label "[@repo]//foo/bar" is synonymous with
                    // "[@repo]//foo/bar:bar". The target name is the last package segment (works
                    // even if `pkg` contains no slash)
                    pkg[pkg.rfind('/').map_or(0, |i| i + 1)..].to_owned()
                }
            };
        }
        Self::validate_and_create(
            repo,
            repo_is_canonical,
            pkg_is_absolute,
            &pkg,
            pkg_ends_with_triple_dots,
            &target,
            raw_label,
        )
    }

    // Java: com/google/devtools/build/lib/cmdline/LabelParser.java:192 LabelParser.Parts.checkPkgIsAbsolute
    pub fn check_pkg_is_absolute(&self) -> Result<(), LabelSyntaxException> {
        if !self.pkg_is_absolute() {
            return Err(syntax_error(format!(
                "invalid label '{}': absolute label must begin with '@' or '//'",
                self.raw()
            )));
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/cmdline/LabelParser.java:198 LabelParser.Parts.checkPkgDoesNotEndWithTripleDots
    pub fn check_pkg_does_not_end_with_triple_dots(&self) -> Result<(), LabelSyntaxException> {
        if self.pkg_ends_with_triple_dots() {
            return Err(syntax_error(format!(
                "invalid label '{}': package name cannot contain '...'",
                self.raw()
            )));
        }
        Ok(())
    }
}

// Java: com/google/devtools/build/lib/cmdline/LabelParser.java:168 LabelParser.Parts.stripTrailingTripleDots
fn strip_trailing_triple_dots(pkg: &str) -> &str {
    if let Some(stripped) = pkg.strip_suffix("/...") {
        return stripped;
    }
    if pkg == "..." {
        return "";
    }
    pkg
}

// Java: com/google/devtools/build/lib/cmdline/LabelParser.java:178 LabelParser.Parts.validateRepoName
fn validate_repo_name(repo: Option<&str>) -> Result<(), LabelSyntaxException> {
    if let Some(repo) = repo {
        RepositoryName::validate(repo)?;
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/cmdline/LabelParser.java:184 LabelParser.Parts.validatePackageName
fn validate_package_name(pkg: &str, target: &str) -> Result<(), LabelSyntaxException> {
    if let Some(pkg_error) = label_validator::validate_package_name(pkg) {
        return Err(syntax_error(format!(
            "invalid package name '{}': {}{}",
            pkg,
            pkg_error,
            perhaps_you_meant_message(pkg, target)
        )));
    }
    Ok(())
}

/// Creates a `LabelSyntaxException` with control characters sanitized.
// Java: com/google/devtools/build/lib/cmdline/LabelParser.java:206 LabelParser.syntaxErrorf
pub(crate) fn syntax_error(message: String) -> LabelSyntaxException {
    LabelSyntaxException::new(sanitize_control_chars(&message))
}

// Java: com/google/devtools/build/lib/cmdline/LabelParser.java:211 LabelParser.perhapsYouMeantMessage
fn perhaps_you_meant_message(pkg: &str, target: &str) -> String {
    if pkg.ends_with(&format!("/{}", target)) {
        format!(" (perhaps you meant \":{}\"?)", target)
    } else {
        String::new()
    }
}

/// Validates a target name, returning it with a trailing "/." removed.
// Java: com/google/devtools/build/lib/cmdline/LabelParser.java:215 LabelParser.validateAndProcessTargetName
pub(crate) fn validate_and_process_target_name(
    pkg: &str,
    target: &str,
    pkg_ends_with_triple_dots: bool,
) -> Result<String, LabelSyntaxException> {
    if pkg_ends_with_triple_dots && target.is_empty() {
        // Allow empty target name if the package part ends in '...'.
        return Ok(target.to_owned());
    }
    if let Some(target_error) = label_validator::validate_target_name(target) {
        return Err(syntax_error(format!(
            "invalid target name '{}': {}{}",
            target,
            target_error,
            perhaps_you_meant_message(pkg, target)
        )));
    }
    // TODO(bazel-team): This should be an error, but we can't make it one for legacy reasons.
    if let Some(stripped) = target.strip_suffix("/.") {
        return Ok(stripped.to_owned());
    }
    Ok(target.to_owned())
}
