//! Port of `lib/runtime/CommandLinePathFactory.java`.
//!
//! PORT: lives here until blaze_runtime can own it (`blaze_repo_fetch` cannot depend on
//! `blaze_runtime`, and the credential helper set-up in this crate's users needs it).

use std::fmt;
use std::sync::LazyLock;

use blaze_vfs::{FileSystemRef, Path, PathFragment};
use indexmap::IndexMap;
use regex::Regex;

use crate::errors::{IoException, IoExceptionKind};

/// The exceptions [`CommandLinePathFactory::create`] throws (all `IOException`s in Java).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandLinePathFactoryError {
    /// `CommandLinePathFactory.CommandLinePathFactoryException`: an exception thrown while
    /// attempting to resolve a path.
    // Java: com/google/devtools/build/lib/runtime/CommandLinePathFactory.java:43 CommandLinePathFactory.CommandLinePathFactoryException
    CommandLinePathFactoryException(String),
    /// `java.io.FileNotFoundException`: no file of that name on `PATH`.
    FileNotFoundException(String),
    /// Any other `IOException` (from `Path.isExecutable`).
    IoException(IoException),
}

impl CommandLinePathFactoryError {
    /// `getMessage()`.
    pub fn get_message(&self) -> String {
        match self {
            CommandLinePathFactoryError::CommandLinePathFactoryException(m)
            | CommandLinePathFactoryError::FileNotFoundException(m) => m.clone(),
            CommandLinePathFactoryError::IoException(e) => e.get_message(),
        }
    }
}

impl fmt::Display for CommandLinePathFactoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_message())
    }
}

impl std::error::Error for CommandLinePathFactoryError {}

impl From<CommandLinePathFactoryError> for IoException {
    /// PORT: `errors::IoExceptionKind` has no `CommandLinePathFactoryException`; it is an
    /// `IOException` with the same message.
    fn from(e: CommandLinePathFactoryError) -> IoException {
        match e {
            CommandLinePathFactoryError::CommandLinePathFactoryException(m) => IoException::new(m),
            CommandLinePathFactoryError::FileNotFoundException(m) => {
                IoException::of(IoExceptionKind::FileNotFoundException, m)
            }
            CommandLinePathFactoryError::IoException(e) => e,
        }
    }
}

/// `Pattern.compile("^(%([a-z_]+)%/+)?([^%].*)$")`, matched with `Matcher.matches()`.
///
/// Java's `.` does not match the line terminators `\n`, `\r`, U+0085, U+2028 and U+2029.
static REPLACEMENT_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("^(%([a-z_]+)%/+)?([^%\n\r\u{85}\u{2028}\u{2029}][^\n\r\u{85}\u{2028}\u{2029}]*)$")
        .unwrap()
});

/// `File.pathSeparator` on Unix.
const PATH_SEPARATOR: char = ':';

/// Factory for creating [`Path`]s from command-line options.
///
/// The difference between this and using `PathFragment::create` directly is that this factory
/// replaces values starting with `%<name>%` with the corresponding (named) roots (e.g.,
/// `%workspace%/foo` becomes `</path/to/workspace>/foo`).
// Java: com/google/devtools/build/lib/runtime/CommandLinePathFactory.java:41 CommandLinePathFactory
#[derive(Clone)]
pub struct CommandLinePathFactory {
    file_system: FileSystemRef,
    roots: IndexMap<String, Path>,
}

impl CommandLinePathFactory {
    /// `@VisibleForTesting public CommandLinePathFactory(FileSystem, ImmutableMap<String, Path>)`.
    // Java: com/google/devtools/build/lib/runtime/CommandLinePathFactory.java:55 CommandLinePathFactory.<init>
    pub fn new(
        file_system: FileSystemRef,
        roots: IndexMap<String, Path>,
    ) -> CommandLinePathFactory {
        CommandLinePathFactory { file_system, roots }
    }

    /// `CommandLinePathFactory.create(fileSystem, directories)`: the factory with the roots
    /// `workspace` and `install_base`, each when the directory is known.
    ///
    /// PORT: Java takes `BlazeDirectories` (in `lib/analysis`, which this crate cannot reach);
    /// the two directories it reads are the parameters (`getWorkspace()`, `getInstallBase()`).
    // Java: com/google/devtools/build/lib/runtime/CommandLinePathFactory.java:60 CommandLinePathFactory.create
    pub fn create_for_directories(
        file_system: FileSystemRef,
        workspace: Option<Path>,
        install_base: Option<Path>,
    ) -> CommandLinePathFactory {
        let mut well_known_roots = IndexMap::new();

        // This is necessary because some tests don't have a workspace set.
        if let Some(workspace) = workspace {
            well_known_roots.insert("workspace".to_string(), workspace);
        }

        if let Some(install_base) = install_base {
            well_known_roots.insert("install_base".to_string(), install_base);
        }

        CommandLinePathFactory::new(file_system, well_known_roots)
    }

    /// Creates a [`Path`].
    ///
    /// Panics (Java `IllegalArgumentException` from `Preconditions.checkArgument`) when `value`
    /// does not match the replacement pattern (e.g. it is empty).
    // Java: com/google/devtools/build/lib/runtime/CommandLinePathFactory.java:81 CommandLinePathFactory.create
    pub fn create(
        &self,
        env: &IndexMap<String, String>,
        value: &str,
    ) -> Result<Path, CommandLinePathFactoryError> {
        let matcher = REPLACEMENT_PATTERN.captures(value);
        blaze_util::check_argument!(matcher.is_some());
        let matcher = matcher.unwrap();

        let root_name = matcher.get(2).map(|m| m.as_str());
        let path = PathFragment::create(matcher.get(3).unwrap().as_str());
        if path.contains_uplevel_references() {
            return Err(
                CommandLinePathFactoryError::CommandLinePathFactoryException(format!(
                    "Path '{value}' must not contain any uplevel references ('..')"
                )),
            );
        }

        // Case 1: `path` is relative to a well-known root.
        if let Some(root_name) = root_name.filter(|r| !r.is_empty()) {
            let Some(root) = self.roots.get(root_name) else {
                return Err(
                    CommandLinePathFactoryError::CommandLinePathFactoryException(format!(
                        "Unknown root {root_name}"
                    )),
                );
            };
            return Ok(root.get_relative(&path));
        }

        // Case 2: `value` is an absolute path.
        if path.is_absolute() {
            return Ok(Path::create(path, &self.file_system));
        }

        // Case 3: `value` is a relative path.
        //
        // Since relative paths from the command-line are ambiguous to where they are relative to
        // (i.e., relative to the workspace?, the directory Bazel is running in? relative to the
        // `.bazelrc` the flag is from?), we only allow relative paths with a single segment
        // (i.e., no `/`) and treat it as relative to the user's `PATH`.
        if path.segment_count() > 1 {
            return Err(
                CommandLinePathFactoryError::CommandLinePathFactoryException(format!(
                    "Path '{value}' must either be absolute or not contain any path separators"
                )),
            );
        }

        let path_variable = env.get("PATH").map(String::as_str).unwrap_or("");
        if !path_variable.is_empty() {
            for lookup_path in path_variable.split(PATH_SEPARATOR) {
                let lookup_path_fragment = PathFragment::create(lookup_path);
                if lookup_path_fragment.is_empty() || !lookup_path_fragment.is_absolute() {
                    // Ignore empty or relative path components. These are uncommon and may be
                    // confusing if bazel is running in a different directory than the user's
                    // current directory.
                    continue;
                }

                let maybe_path =
                    Path::create(lookup_path_fragment, &self.file_system).get_relative(&path);
                if maybe_path.exists()
                    && maybe_path.is_file()
                    && maybe_path
                        .is_executable()
                        .map_err(|e| CommandLinePathFactoryError::IoException(e.into()))?
                {
                    return Ok(maybe_path);
                }
            }
        }

        Err(CommandLinePathFactoryError::FileNotFoundException(format!(
            "Could not find file with name '{}' on PATH '{}'",
            path.get_path_string(),
            path_variable
        )))
    }
}
