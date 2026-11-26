//! Factory for creating paths from command-line options.
//!
//! PORT: Java's class is `lib/runtime/CommandLinePathFactory`; it is ported here, below
//! `blaze_runtime`, because its only users are `GoogleAuthUtils.newCredentialHelperProvider`'s
//! callers (the remote module, the downloader setup), which cannot depend on `blaze_runtime`.

use std::sync::OnceLock;

use blaze_util::java_regex::Pattern;
use blaze_vfs::{FileSystemRef, Path, PathFragment, Symlinks};
use indexmap::IndexMap;

use crate::errors::{IoException, IoExceptionKind};

// Java: com/google/devtools/build/lib/runtime/CommandLinePathFactory.java:47 CommandLinePathFactory.REPLACEMENT_PATTERN
fn replacement_pattern() -> &'static Pattern {
    static PATTERN: OnceLock<Pattern> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Pattern::compile("^(%([a-z_]+)%/+)?([^%].*)$", 0)
            .expect("CommandLinePathFactory.REPLACEMENT_PATTERN compiles")
    })
}

/// Factory for creating [`PathFragment`]s from command-line options.
///
/// The difference between this and using `PathFragment::create` directly is that this factory
/// replaces values starting with `%<name>%` with the corresponding (named) roots (e.g.,
/// `%workspace%/foo` becomes `</path/to/workspace>/foo`).
// Java: com/google/devtools/build/lib/runtime/CommandLinePathFactory.java:38 CommandLinePathFactory
#[derive(Clone, Debug)]
pub struct CommandLinePathFactory {
    file_system: FileSystemRef,
    roots: IndexMap<String, Path>,
}

impl CommandLinePathFactory {
    // Java: com/google/devtools/build/lib/runtime/CommandLinePathFactory.java:55 CommandLinePathFactory.<init>
    pub fn new(
        file_system: FileSystemRef,
        roots: IndexMap<String, Path>,
    ) -> CommandLinePathFactory {
        CommandLinePathFactory { file_system, roots }
    }

    /// `create(fileSystem, directories)`, given `directories.getWorkspace()` and
    /// `directories.getInstallBase()`.
    // Java: com/google/devtools/build/lib/runtime/CommandLinePathFactory.java:60 CommandLinePathFactory.create
    pub fn create_for(
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
    // Java: com/google/devtools/build/lib/runtime/CommandLinePathFactory.java:80 CommandLinePathFactory.create
    pub fn create(&self, env: &IndexMap<String, String>, value: &str) -> Result<Path, IoException> {
        let pattern = replacement_pattern();
        blaze_util::check_argument!(pattern.matches(value));
        let captures = pattern
            .regex()
            .captures(value)
            .expect("a matching value has captures");

        let root_name = captures.get(2).map(|m| m.as_str());
        let path = PathFragment::create(captures.get(3).map_or("", |m| m.as_str()));
        if path.contains_uplevel_references() {
            return Err(IoException::of(
                IoExceptionKind::CommandLinePathFactoryException,
                format!("Path '{value}' must not contain any uplevel references ('..')"),
            ));
        }

        // Case 1: `path` is relative to a well-known root.
        if let Some(root_name) = root_name.filter(|r| !r.is_empty()) {
            let Some(root) = self.roots.get(root_name) else {
                return Err(IoException::of(
                    IoExceptionKind::CommandLinePathFactoryException,
                    format!("Unknown root {root_name}"),
                ));
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
            return Err(IoException::of(
                IoExceptionKind::CommandLinePathFactoryException,
                format!(
                    "Path '{value}' must either be absolute or not contain any path separators"
                ),
            ));
        }

        let path_variable = env.get("PATH").map(String::as_str).unwrap_or("");
        if !path_variable.is_empty() {
            // PATH_SPLITTER: Splitter.on(File.pathSeparator).
            for lookup_path in path_variable.split(':') {
                let lookup_path_fragment = PathFragment::create(lookup_path);
                if lookup_path_fragment.is_empty() || !lookup_path_fragment.is_absolute() {
                    // Ignore empty or relative path components. These are uncommon and may be
                    // confusing if bazel is running in a different directory than the user's
                    // current directory.
                    continue;
                }

                let maybe_path =
                    Path::create(lookup_path_fragment, &self.file_system).get_relative(&path);
                if maybe_path.exists_with(Symlinks::Follow)
                    && maybe_path.is_file_with(Symlinks::Follow)
                    && maybe_path.is_executable().map_err(IoException::from)?
                {
                    return Ok(maybe_path);
                }
            }
        }

        Err(IoException::of(
            IoExceptionKind::FileNotFoundException,
            format!(
                "Could not find file with name '{}' on PATH '{}'",
                path.get_path_string(),
                path_variable
            ),
        ))
    }
}
