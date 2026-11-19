//! Port of the client's rc-file reader (`src/main/cpp/rc_file.cc`).

use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;

use blaze_util::string_encoding::{internal_to_platform, latin1_decode, platform_to_internal};

use super::sem_ver::SemVer;
use super::tokenize::{join_path, replace, strip_whitespace, tokenize};

// Java: src/main/cpp/rc_file.cc:43 kCommandImport
const COMMAND_IMPORT: &str = "import";
// Java: src/main/cpp/rc_file.cc:44 kCommandTryImport
const COMMAND_TRY_IMPORT: &str = "try-import";
// Java: src/main/cpp/rc_file.cc:45 kCommandTryImportIfBazelVersion
const COMMAND_TRY_IMPORT_IF_BAZEL_VERSION: &str = "try-import-if-bazel-version";

// Java: src/main/cpp/rc_file.cc:60 kBazelVersionCmpOp
// RE2's `\S` is `[^\t\n\f\r ]` (ASCII whitespace only); Rust's is Unicode, which would
// reject U+0085 and U+00A0, chars of an internal string.
static BAZEL_VERSION_CMP_OP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(<=?|>=?|==|!=|~)([^\t\n\x0C\r ]+)$").unwrap());

// Java: src/main/cpp/workspace_layout.h:60 WorkspaceLayout::kWorkspacePrefix
pub const WORKSPACE_PREFIX: &str = "%workspace%/";

// Java: src/main/cpp/workspace_layout.cc:73 WorkspaceLayout::ResolveWorkspaceRelativeRcFilePath
/// Resolves a `%workspace%/...` import path; `None` if the resolved path does not exist.
pub fn resolve_workspace_relative_rc_file_path(
    workspace: &str,
    import_path: &str,
) -> Option<String> {
    assert!(import_path.starts_with(WORKSPACE_PREFIX));
    let resolved_path = join_path(workspace, &import_path[WORKSPACE_PREFIX.len()..]);
    // Java: src/main/cpp/util/file_posix.cc:372 PathExists (`access(path, F_OK)` on the raw
    // bytes): the internal string is the bytes of the path.
    if std::path::Path::new(&*internal_to_platform(&resolved_path)).exists() {
        Some(resolved_path)
    } else {
        None
    }
}

// Java: src/main/cpp/rc_file.h:30 RcOption
/// A single option in an rc file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RcOption {
    pub option: String,
    /// Index into [`RcFile::canonical_source_paths`].
    pub source_index: usize,
}

// Java: src/main/cpp/rc_file.h:41 RcFile::ParseError
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    None,
    UnreadableFile,
    InvalidFormat,
    ImportLoop,
}

/// Reads a file: `Ok(contents)` or `Err(error message)`.
pub type ReadFileFn<'a> = &'a dyn Fn(&str) -> Result<String, String>;
/// Canonicalizes a path (`blaze_util::MakeCanonical`).
pub type CanonicalizePathFn<'a> = &'a dyn Fn(&str) -> String;

// Java: src/main/cpp/rc_file.cc:261 RcFile::ReadFileDefault
pub fn read_file_default(filename: &str) -> Result<String, String> {
    // The C++ ReadFile keeps the rc file's raw bytes in a std::string: one char per byte.
    std::fs::read(std::path::Path::new(&*internal_to_platform(filename)))
        .map(|b| latin1_decode(&b).into_owned())
        .map_err(|e| {
            // Java: src/main/cpp/util/errors_posix.cc:31 blaze_util::GetLastErrorString
            // "(error: <errno>): <strerror(errno)>".
            match e.raw_os_error() {
                Some(code) => format!(
                    "(error: {}): {}",
                    code,
                    std::io::Error::from_raw_os_error(code)
                        .to_string()
                        .split(" (os error")
                        .next()
                        .unwrap_or("")
                ),
                None => e.to_string(),
            }
        })
}

// Java: src/main/cpp/util/file_posix.cc:378 blaze_util::MakeCanonical
/// `realpath`, or "" on failure.
pub fn make_canonical(path: &str) -> String {
    // The C++ MakeCanonical returns realpath's raw bytes.
    std::fs::canonicalize(std::path::Path::new(&*internal_to_platform(path)))
        .map(|p| platform_to_internal(p.as_os_str()).into_owned())
        .unwrap_or_default()
}

// Java: src/main/cpp/rc_file.h:35 RcFile
/// A parsed rc file with all its imports.
#[derive(Clone, Debug, Default)]
pub struct RcFile {
    canonical_rcfile_paths: Vec<String>,
    /// Command -> all options for that command, in order of appearance.
    /// PORT: Java (C++) uses an absl::flat_hash_map whose command order is unspecified; we keep
    /// first-appearance order. The server only depends on per-command order.
    options: IndexMap<String, Vec<RcOption>>,
}

impl RcFile {
    // Java: src/main/cpp/rc_file.cc:62 RcFile::Parse
    /// Parses `filename` and its imports. On failure returns the error kind and error text.
    pub fn parse(
        filename: &str,
        workspace: &str,
        build_label: &str,
        sem_ver: Option<&SemVer>,
        read_file: ReadFileFn<'_>,
        canonicalize_path: CanonicalizePathFn<'_>,
    ) -> Result<RcFile, (ParseError, String)> {
        let mut rcfile = RcFile::default();
        let mut import_stack = vec![filename.to_string()];
        let mut error_text = String::new();
        let error = rcfile.parse_file(
            filename,
            workspace,
            build_label,
            sem_ver,
            read_file,
            canonicalize_path,
            &mut import_stack,
            &mut error_text,
        );
        if error == ParseError::None {
            Ok(rcfile)
        } else {
            Err((error, error_text))
        }
    }

    /// [`RcFile::parse`] with the default file reader and canonicalizer and no build label.
    pub fn parse_default(filename: &str, workspace: &str) -> Result<RcFile, (ParseError, String)> {
        Self::parse(
            filename,
            workspace,
            "",
            None,
            &read_file_default,
            &make_canonical,
        )
    }

    /// All rc sources of this file (itself first, then imports in order).
    pub fn canonical_source_paths(&self) -> &[String] {
        &self.canonical_rcfile_paths
    }

    pub fn options(&self) -> &IndexMap<String, Vec<RcOption>> {
        &self.options
    }

    // Java: src/main/cpp/rc_file.cc:85 RcFile::ParseFile
    fn parse_file(
        &mut self,
        filename: &str,
        workspace: &str,
        build_label: &str,
        sem_ver: Option<&SemVer>,
        read_file: ReadFileFn<'_>,
        canonicalize_path: CanonicalizePathFn<'_>,
        import_stack: &mut Vec<String>,
        error_text: &mut String,
    ) -> ParseError {
        let mut contents = match read_file(filename) {
            Ok(c) => c,
            Err(error_msg) => {
                *error_text = format!(
                    "Unexpected error reading config file '{}': {}",
                    filename, error_msg
                );
                return ParseError::UnreadableFile;
            }
        };
        let canonical_filename = canonicalize_path(filename);

        let rcfile_index = self.canonical_rcfile_paths.len();
        self.canonical_rcfile_paths.push(canonical_filename.clone());

        // A '\' at the end of a line continues the line.
        replace("\\\r\n", "", &mut contents);
        replace("\\\n", "", &mut contents);

        for line in contents.split('\n') {
            let mut line = line.to_string();
            strip_whitespace(&mut line);
            if line.is_empty() {
                continue;
            }
            let words = tokenize(&line, '#');
            if words.is_empty() {
                continue;
            }
            let command = words[0].as_str();
            if command != COMMAND_IMPORT
                && command != COMMAND_TRY_IMPORT
                && command != COMMAND_TRY_IMPORT_IF_BAZEL_VERSION
            {
                // `options_[command].push_back(...)` per word: an argument-less command line
                // creates no entry.
                for word in &words[1..] {
                    self.options
                        .entry(command.to_string())
                        .or_default()
                        .push(RcOption {
                            option: word.clone(),
                            source_index: rcfile_index,
                        });
                }
                continue;
            }

            if (command == COMMAND_TRY_IMPORT_IF_BAZEL_VERSION && words.len() != 3)
                || (command != COMMAND_TRY_IMPORT_IF_BAZEL_VERSION && words.len() != 2)
            {
                *error_text = format!(
                    "Invalid import declaration in config file '{}': '{}'",
                    canonical_filename, line
                );
                return ParseError::InvalidFormat;
            }

            let mut import_filename;
            if command == COMMAND_IMPORT || command == COMMAND_TRY_IMPORT {
                import_filename = words[1].clone();
            } else {
                let Some(sem_ver) = sem_ver else {
                    // BAZEL_LOG(INFO): "Skipping '%s' import because bazel build label '%s' is
                    // not a valid semantic version."
                    let _ = build_label;
                    continue;
                };
                let conditional = &words[1];
                import_filename = words[2].clone();
                if let Some(m) = BAZEL_VERSION_CMP_OP.captures(conditional) {
                    match bazel_version_matches_condition(sem_ver, &m[1], &m[2]) {
                        Err(e) => {
                            *error_text = format!(
                                "Invalid import declaration in config file '{}': '{}'. {}",
                                canonical_filename, line, e
                            );
                            return ParseError::InvalidFormat;
                        }
                        Ok(false) => {
                            // BAZEL_LOG(INFO): skipped optional import.
                            continue;
                        }
                        Ok(true) => {}
                    }
                } else {
                    *error_text = format!(
                        "Invalid version condition in config file '{}': '{}'. Condition '{}'. A \
                         valid condition is one of the following 7 comparison operators ('<', \
                         '<=', '>', '>=', '==', '!=', '~') followed by a semantic version.",
                        canonical_filename, line, conditional
                    );
                    return ParseError::InvalidFormat;
                }
            }

            if import_filename.starts_with(WORKSPACE_PREFIX) {
                match resolve_workspace_relative_rc_file_path(workspace, &import_filename) {
                    None => {
                        if command == COMMAND_IMPORT {
                            *error_text = format!(
                                "Nonexistent path in import declaration in config file '{}': \
                                 '{}' (are you in your source checkout/WORKSPACE?)",
                                canonical_filename, line
                            );
                            return ParseError::InvalidFormat;
                        }
                        // For try-import, we ignore it if we couldn't find a file.
                        continue;
                    }
                    Some(resolved) => import_filename = resolved,
                }
            }

            if import_stack.contains(&import_filename) {
                let mut lp = String::new();
                for imported_rc in import_stack.iter() {
                    lp.push_str(&format!("  {}\n", imported_rc));
                }
                lp.push_str(&format!("  {}\n", import_filename));
                *error_text = format!("Import loop detected:\n{}", lp);
                return ParseError::ImportLoop;
            }

            import_stack.push(import_filename.clone());
            let parse_error = self.parse_file(
                &import_filename,
                workspace,
                build_label,
                sem_ver,
                read_file,
                canonicalize_path,
                import_stack,
                error_text,
            );
            if parse_error != ParseError::None {
                if parse_error == ParseError::UnreadableFile
                    && (command == COMMAND_TRY_IMPORT
                        || command == COMMAND_TRY_IMPORT_IF_BAZEL_VERSION)
                {
                    // For try-import.*, we ignore it if we couldn't find a file.
                    error_text.clear();
                } else {
                    return parse_error;
                }
            }
            import_stack.pop();
        }
        ParseError::None
    }
}

// Java: src/main/cpp/rc_file.cc:270 BazelVersionMatchesCondition
/// Whether `build_label` satisfies `op compare_version`; `Err(error_text)` on a bad version.
pub fn bazel_version_matches_condition(
    build_label: &SemVer,
    op: &str,
    compare_version: &str,
) -> Result<bool, String> {
    if op == "~" {
        let num_dots = compare_version.matches('.').count();
        let semver_compare_version = match num_dots {
            0 => SemVer::parse(&format!("{}.0.0", compare_version)),
            1 => SemVer::parse(&format!("{}.0", compare_version)),
            _ => SemVer::parse(compare_version),
        };
        let Some(v) = semver_compare_version else {
            return Err(format!(
                "Could not parse the tilde range version '{}' as a valid semantic version.",
                compare_version
            ));
        };
        if num_dots == 0 {
            return Ok(*build_label >= v && *build_label < v.next_major_version());
        }
        return Ok(*build_label >= v && *build_label < v.next_minor_version());
    }
    let Some(v) = SemVer::parse(compare_version) else {
        return Err(format!(
            "Could not parse version '{}' as a valid semantic version.",
            compare_version
        ));
    };
    match op {
        "<" => Ok(*build_label < v),
        "<=" => Ok(*build_label <= v),
        ">" => Ok(*build_label > v),
        ">=" => Ok(*build_label >= v),
        "==" => Ok(*build_label == v),
        "!=" => Ok(*build_label != v),
        _ => Err(format!("Invalid comparison operator '{}'.", op)),
    }
}
