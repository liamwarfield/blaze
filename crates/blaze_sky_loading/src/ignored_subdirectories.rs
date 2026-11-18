//! IGNORED_SUBDIRECTORIES: `.bazelignore` and REPO.bazel `ignore_directories()`.
//!
//! Java: com/google/devtools/build/lib/skyframe/{IgnoredSubdirectoriesFunction,
//! IgnoredSubdirectoriesValue}.java and com/google/devtools/build/lib/cmdline/
//! IgnoredSubdirectories.java.

use std::fmt;
use std::hash::{Hash, Hasher};

use blaze_cmdline::RepositoryName;
use blaze_cmdline::repository_name::MAIN;
use blaze_skyframe::{ComputeError, Environment, Key, SkyFunctionException, SkyFunctionName};
use blaze_vfs::{PathFragment, RootedPath};
use indexmap::IndexSet;

use crate::file::FileKey;
use crate::io::IOException;
use crate::precomputed::PATH_PACKAGE_LOCATOR;
use crate::repo_file::{BadRepoFileException, RepoFileKey};
use crate::repos::{RepositoryDirectoryKey, RepositoryDirectoryValue};
use crate::sky_functions;
use crate::unix_glob;

/// A set of subdirectories to ignore during target pattern matching or globbing.
///
/// PORT: move to blaze_cmdline (Java `cmdline/IgnoredSubdirectories`); it lives here until
/// blaze_cmdline can use `UnixGlob`'s matchers.
// Java: com/google/devtools/build/lib/cmdline/IgnoredSubdirectories.java:38 IgnoredSubdirectories
#[derive(Clone, PartialEq, Eq)]
pub struct IgnoredSubdirectories {
    prefixes: IndexSet<PathFragment>,
    // We keep the split version because that's faster to match and the non-split one because that
    // allows for simpler equality checking.
    patterns: Vec<String>,
}

impl IgnoredSubdirectories {
    // Java: com/google/devtools/build/lib/cmdline/IgnoredSubdirectories.java:39 IgnoredSubdirectories.EMPTY
    pub fn empty() -> IgnoredSubdirectories {
        IgnoredSubdirectories {
            prefixes: IndexSet::new(),
            patterns: Vec::new(),
        }
    }

    fn split_pattern(p: &str) -> Vec<String> {
        p.split('/').map(String::from).collect()
    }

    // Java: com/google/devtools/build/lib/cmdline/IgnoredSubdirectories.java:85 IgnoredSubdirectories.of
    pub fn of(prefixes: IndexSet<PathFragment>, patterns: Vec<String>) -> IgnoredSubdirectories {
        if prefixes.is_empty() && patterns.is_empty() {
            return Self::empty();
        }
        for prefix in &prefixes {
            blaze_util::check_argument!(!prefix.is_absolute());
        }
        IgnoredSubdirectories { prefixes, patterns }
    }

    // Java: com/google/devtools/build/lib/cmdline/IgnoredSubdirectories.java:98 IgnoredSubdirectories.withPrefix
    pub fn with_prefix(&self, prefix: &PathFragment) -> IgnoredSubdirectories {
        blaze_util::check_argument!(!prefix.is_absolute());
        let prefixed_prefixes = self
            .prefixes
            .iter()
            .map(|p| prefix.get_relative(p))
            .collect();
        let prefixed_patterns = self
            .patterns
            .iter()
            .map(|p| format!("{}/{}", prefix, p))
            .collect();
        IgnoredSubdirectories {
            prefixes: prefixed_prefixes,
            patterns: prefixed_patterns,
        }
    }

    // Java: com/google/devtools/build/lib/cmdline/IgnoredSubdirectories.java:110 IgnoredSubdirectories.union
    pub fn union(&self, other: &IgnoredSubdirectories) -> IgnoredSubdirectories {
        let mut prefixes = self.prefixes.clone();
        prefixes.extend(other.prefixes.iter().cloned());
        let mut patterns: IndexSet<String> = self.patterns.iter().cloned().collect();
        patterns.extend(other.patterns.iter().cloned());
        IgnoredSubdirectories {
            prefixes,
            patterns: patterns.into_iter().collect(),
        }
    }

    /// Filters out entries that cannot match anything under `directory`.
    // Java: com/google/devtools/build/lib/cmdline/IgnoredSubdirectories.java:118 IgnoredSubdirectories.filterForDirectory
    pub fn filter_for_directory(&self, directory: &PathFragment) -> IgnoredSubdirectories {
        let filtered_prefixes = self
            .prefixes
            .iter()
            .filter(|p| p.starts_with(directory))
            .cloned()
            .collect();
        let split_directory: Vec<String> = directory.segments().map(String::from).collect();
        let filtered_patterns = self
            .patterns
            .iter()
            .filter(|p| unix_glob::can_match_child(&Self::split_pattern(p), &split_directory))
            .cloned()
            .collect();
        IgnoredSubdirectories {
            prefixes: filtered_prefixes,
            patterns: filtered_patterns,
        }
    }

    // Java: com/google/devtools/build/lib/cmdline/IgnoredSubdirectories.java:133 IgnoredSubdirectories.prefixes
    pub fn prefixes(&self) -> &IndexSet<PathFragment> {
        &self.prefixes
    }

    // Java: com/google/devtools/build/lib/cmdline/IgnoredSubdirectories.java:137 IgnoredSubdirectories.isEmpty
    pub fn is_empty(&self) -> bool {
        self.prefixes.is_empty() && self.patterns.is_empty()
    }

    /// Checks whether every path in this instance can conceivably match something under
    /// `directory`.
    // Java: com/google/devtools/build/lib/cmdline/IgnoredSubdirectories.java:145 IgnoredSubdirectories.allPathsAreUnder
    pub fn all_paths_are_under(&self, directory: &PathFragment) -> bool {
        for prefix in &self.prefixes {
            if !prefix.starts_with(directory) {
                return false;
            }
            if prefix == directory {
                return false;
            }
        }
        true
    }

    /// Returns the entry that matches a given directory or `None` if none.
    // Java: com/google/devtools/build/lib/cmdline/IgnoredSubdirectories.java:160 IgnoredSubdirectories.matchingEntry
    pub fn matching_entry(&self, directory: &PathFragment) -> Option<String> {
        for prefix in &self.prefixes {
            if directory.starts_with(prefix) {
                return Some(prefix.get_path_string().to_owned());
            }
        }
        let segment_array: Vec<String> = directory.segments().map(String::from).collect();
        for pattern in &self.patterns {
            if unix_glob::matches_prefix(&Self::split_pattern(pattern), &segment_array) {
                return Some(pattern.clone());
            }
        }
        None
    }
}

// Java: com/google/devtools/build/lib/cmdline/IgnoredSubdirectories.java:192 IgnoredSubdirectories.hashCode
impl Hash for IgnoredSubdirectories {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Set equality ignores order, so hash an order-independent summary of the prefixes.
        self.prefixes.len().hash(state);
        self.patterns.hash(state);
    }
}

// Java: com/google/devtools/build/lib/cmdline/IgnoredSubdirectories.java:197 IgnoredSubdirectories.toString
impl fmt::Debug for IgnoredSubdirectories {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prefixes: Vec<String> = self.prefixes.iter().map(|p| p.to_string()).collect();
        write!(
            f,
            "IgnoredSubdirectories{{prefixes=[{}], patterns=[{}]}}",
            prefixes.join(", "),
            self.patterns.join(", ")
        )
    }
}

/// An immutable set of package name prefixes that should be ignored.
// Java: com/google/devtools/build/lib/skyframe/IgnoredSubdirectoriesValue.java:33 IgnoredSubdirectoriesValue
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IgnoredSubdirectoriesValue {
    ignored_subdirectories: IgnoredSubdirectories,
}

impl IgnoredSubdirectoriesValue {
    // Java: com/google/devtools/build/lib/skyframe/IgnoredSubdirectoriesValue.java:37 IgnoredSubdirectoriesValue.EMPTY
    pub fn empty() -> IgnoredSubdirectoriesValue {
        IgnoredSubdirectoriesValue {
            ignored_subdirectories: IgnoredSubdirectories::empty(),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/IgnoredSubdirectoriesValue.java:45 IgnoredSubdirectoriesValue.of
    pub fn of(prefixes: IndexSet<PathFragment>, patterns: Vec<String>) -> Self {
        if prefixes.is_empty() && patterns.is_empty() {
            Self::empty()
        } else {
            IgnoredSubdirectoriesValue {
                ignored_subdirectories: IgnoredSubdirectories::of(prefixes, patterns),
            }
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/IgnoredSubdirectoriesValue.java:52 IgnoredSubdirectoriesValue.of
    pub fn of_ignored(ignored_subdirectories: IgnoredSubdirectories) -> Self {
        if ignored_subdirectories.is_empty() {
            Self::empty()
        } else {
            IgnoredSubdirectoriesValue {
                ignored_subdirectories,
            }
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/IgnoredSubdirectoriesValue.java:69 IgnoredSubdirectoriesValue.asIgnoredSubdirectories
    pub fn as_ignored_subdirectories(&self) -> &IgnoredSubdirectories {
        &self.ignored_subdirectories
    }
}

/// Key of IGNORED_SUBDIRECTORIES: a repository. `Debug` prints `IGNORED_SUBDIRECTORIES:@@name`.
// Java: com/google/devtools/build/lib/skyframe/IgnoredSubdirectoriesValue.java:94 IgnoredSubdirectoriesValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct IgnoredSubdirectoriesKey(pub RepositoryName);

impl IgnoredSubdirectoriesKey {
    /// Creates a key from the main repository.
    // Java: com/google/devtools/build/lib/skyframe/IgnoredSubdirectoriesValue.java:59 IgnoredSubdirectoriesValue.key
    pub fn key_main() -> Self {
        IgnoredSubdirectoriesKey(MAIN.clone())
    }

    /// Creates a key from the given repository name.
    // Java: com/google/devtools/build/lib/skyframe/IgnoredSubdirectoriesValue.java:64 IgnoredSubdirectoriesValue.key
    pub fn key(repository: RepositoryName) -> Self {
        IgnoredSubdirectoriesKey(repository)
    }
}

impl fmt::Debug for IgnoredSubdirectoriesKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", Self::FUNCTION, self.0)
    }
}

impl Key for IgnoredSubdirectoriesKey {
    type Value = IgnoredSubdirectoriesValue;
    const FUNCTION: SkyFunctionName = sky_functions::IGNORED_SUBDIRECTORIES;
}

/// Exception thrown when an ignore path is wrong for some reason.
// Java: com/google/devtools/build/lib/skyframe/IgnoredSubdirectoriesValue.java:117 IgnoredSubdirectoriesValue.InvalidIgnorePathException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("Invalid path in {path}: {message}")]
pub struct InvalidIgnorePathException {
    path: String,
    message: String,
}

impl InvalidIgnorePathException {
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        InvalidIgnorePathException {
            path: path.into(),
            message: message.into(),
        }
    }
}

/// Repository-relative path of the bazelignore file.
// Java: com/google/devtools/build/lib/skyframe/IgnoredSubdirectoriesFunction.java:58 IgnoredSubdirectoriesFunction.BAZELIGNORE_REPOSITORY_RELATIVE_PATH
pub const BAZELIGNORE_REPOSITORY_RELATIVE_PATH: &str = ".bazelignore";

// Java: com/google/devtools/build/lib/skyframe/IgnoredSubdirectoriesFunction.java:225 IgnoredSubdirectoriesFunction.IgnoredSubdirectoriesFunctionException
enum IgnoredSubdirectoriesFunctionException {
    InconsistentFilesystem(IOException),
    InvalidIgnorePath(InvalidIgnorePathException),
    Io(IOException),
    BadRepoFile(BadRepoFileException),
}

impl From<IgnoredSubdirectoriesFunctionException> for ComputeError {
    fn from(e: IgnoredSubdirectoriesFunctionException) -> ComputeError {
        match e {
            IgnoredSubdirectoriesFunctionException::InconsistentFilesystem(e)
            | IgnoredSubdirectoriesFunctionException::Io(e) => {
                SkyFunctionException::transient(e).into()
            }
            IgnoredSubdirectoriesFunctionException::InvalidIgnorePath(e) => {
                SkyFunctionException::persistent(e).into()
            }
            IgnoredSubdirectoriesFunctionException::BadRepoFile(e) => {
                SkyFunctionException::persistent(e).into()
            }
        }
    }
}

/// Reads `pattern_file` and adds its ignored prefixes.
// Java: com/google/devtools/build/lib/skyframe/IgnoredSubdirectoriesFunction.java:80 IgnoredSubdirectoriesFunction.getIgnoredPrefixes
fn get_ignored_prefixes(
    pattern_file: &RootedPath,
    ignored_directories_builder: &mut IndexSet<PathFragment>,
) -> Result<(), IgnoredSubdirectoriesFunctionException> {
    let path = pattern_file.as_path();
    let read = (|| -> std::io::Result<String> {
        let mut input = path.get_input_stream()?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut input, &mut bytes)?;
        // PORT: Java decodes with an InputStreamReader (UTF-8, malformed input replaced).
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    })();
    let contents = match read {
        Ok(c) => c,
        Err(e) => {
            let error_message = format!("error '{}'", e);
            return Err(
                IgnoredSubdirectoriesFunctionException::InconsistentFilesystem(
                    IOException::inconsistent_filesystem(format!(
                        "{} is not readable because: {}. Was it modified mid-build?",
                        path, error_message
                    )),
                ),
            );
        }
    };
    // Java: PathFragmentLineProcessor over CharStreams.readLines (splits on \n, \r\n and \r).
    let mut fragments: IndexSet<PathFragment> = IndexSet::new();
    for line in read_lines(&contents) {
        if !line.is_empty() && !line.starts_with('#') {
            fragments.insert(PathFragment::create(line));
            // Java also validates the line with java.nio.file.Path.of(line), which only rejects
            // NUL on Unix.
            if line.contains('\0') {
                return Err(IgnoredSubdirectoriesFunctionException::InvalidIgnorePath(
                    InvalidIgnorePathException::new(
                        path.to_string(),
                        format!("Nul character not allowed: {}", line),
                    ),
                ));
            }
        }
    }
    for ignored in fragments {
        if ignored.is_absolute() {
            return Err(IgnoredSubdirectoriesFunctionException::InvalidIgnorePath(
                InvalidIgnorePathException::new(
                    path.to_string(),
                    format!("'{}': cannot be an absolute path", ignored),
                ),
            ));
        }
        ignored_directories_builder.insert(ignored);
    }
    Ok(())
}

/// Guava `CharStreams.readLines`: lines split on `\n`, `\r\n` or `\r`, without terminators; no
/// trailing empty line.
fn read_lines(contents: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let bytes = contents.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                lines.push(&contents[start..i]);
                i += 1;
                start = i;
            }
            b'\r' => {
                lines.push(&contents[start..i]);
                i += 1;
                if i < bytes.len() && bytes[i] == b'\n' {
                    i += 1;
                }
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < bytes.len() {
        lines.push(&contents[start..]);
    }
    lines
}

// Java: com/google/devtools/build/lib/skyframe/IgnoredSubdirectoriesFunction.java:110 IgnoredSubdirectoriesFunction.computeIgnoredPatterns
fn compute_ignored_patterns(
    env: &mut Environment<'_>,
    repository_name: &RepositoryName,
) -> Result<Option<Vec<String>>, ComputeError> {
    let key = RepoFileKey::key(repository_name.clone());
    // Java: getValueOrThrow(key, IOException.class, BadRepoFileException.class)
    let repo_file_value = match env.get_value_or_throw_with(key, |e| {
        if let Some(e) = e.downcast_ref::<IOException>() {
            Some(IgnoredSubdirectoriesFunctionException::Io(e.clone()))
        } else {
            e.downcast_ref::<BadRepoFileException>()
                .map(|e| IgnoredSubdirectoriesFunctionException::BadRepoFile(e.clone()))
        }
    })? {
        Err(e) => return Err(e.into()),
        Ok(v) => v,
    };
    if env.values_missing() {
        return Ok(None);
    }
    Ok(Some(
        repo_file_value
            .expect("not missing")
            .ignored_directories()
            .to_vec(),
    ))
}

// Java: com/google/devtools/build/lib/skyframe/IgnoredSubdirectoriesFunction.java:128 IgnoredSubdirectoriesFunction.computeIgnoredPrefixes
fn compute_ignored_prefixes(
    env: &mut Environment<'_>,
    repository_name: &RepositoryName,
) -> Result<Option<IndexSet<PathFragment>>, ComputeError> {
    let mut ignored_prefixes_builder: IndexSet<PathFragment> = IndexSet::new();
    let pkg_locator = PATH_PACKAGE_LOCATOR.get(env)?;
    if env.values_missing() {
        return Ok(None);
    }
    let pkg_locator = pkg_locator.expect("not missing");
    if repository_name.is_main() {
        // PORT: --vendor_dir (VENDOR_DIRECTORY) is not supported yet; Blaze never vendors.
        for package_path_entry in pkg_locator.get_path_entries() {
            let rooted_prefix_file = RootedPath::to_rooted_path(
                package_path_entry.clone(),
                &PathFragment::create(BAZELIGNORE_REPOSITORY_RELATIVE_PATH),
            );
            let Some(prefix_file_value) =
                env.get_value(FileKey::key(rooted_prefix_file.clone()))?
            else {
                return Ok(None);
            };
            if prefix_file_value.is_file() {
                get_ignored_prefixes(&rooted_prefix_file, &mut ignored_prefixes_builder)?;
                break;
            }
        }
    } else {
        // Make sure the repository is fetched.
        let Some(repository_value) =
            env.get_value(RepositoryDirectoryKey::key(repository_name.clone()))?
        else {
            return Ok(None);
        };
        if let RepositoryDirectoryValue::Success { root, .. } = &*repository_value {
            let rooted_prefix_file = RootedPath::to_rooted_path(
                root.clone(),
                &PathFragment::create(BAZELIGNORE_REPOSITORY_RELATIVE_PATH),
            );
            let Some(prefix_file_value) =
                env.get_value(FileKey::key(rooted_prefix_file.clone()))?
            else {
                return Ok(None);
            };
            if prefix_file_value.is_file() {
                get_ignored_prefixes(&rooted_prefix_file, &mut ignored_prefixes_builder)?;
            }
        }
    }
    Ok(Some(ignored_prefixes_builder))
}

/// A SkyFunction for [`IgnoredSubdirectoriesValue`]: which directories should be ignored in a
/// package, from the `.bazelignore` file or the `ignore_directories()` function in REPO.bazel.
// Java: com/google/devtools/build/lib/skyframe/IgnoredSubdirectoriesFunction.java:182 IgnoredSubdirectoriesFunction.compute
pub fn ignored_subdirectories_function(
    key: &IgnoredSubdirectoriesKey,
    env: &mut Environment<'_>,
) -> Result<Option<IgnoredSubdirectoriesValue>, ComputeError> {
    let repository_name = &key.0;
    let ignored_patterns = compute_ignored_patterns(env, repository_name)?;
    if env.values_missing() {
        return Ok(None);
    }
    let ignored_prefixes = compute_ignored_prefixes(env, repository_name)?;
    if env.values_missing() {
        return Ok(None);
    }
    Ok(Some(IgnoredSubdirectoriesValue::of(
        ignored_prefixes.expect("not missing"),
        ignored_patterns.expect("not missing"),
    )))
}
