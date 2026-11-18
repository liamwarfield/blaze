//! Port of `PathPackageLocator` (the parts Skyframe loading reads).
//!
//! Java: com/google/devtools/build/lib/pkgcache/PathPackageLocator.java. Not ported yet:
//! `getWorkspaceFile`.

use std::fmt;
use std::hash::{Hash, Hasher};

use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::label_constants::EXTERNAL_REPOSITORY_LOCATION;
use blaze_vfs::{DirentType, Path, PathFragment, Root, Symlinks};

use crate::build_file_name::BuildFileName;

// Java: com/google/devtools/build/lib/pkgcache/PathPackageLocator.java:51 PathPackageLocator.WORKSPACE_WILDCARD
const WORKSPACE_WILDCARD: &str = "%workspace%";

/// A mapping from the name of a package to the location of its BUILD file. The implementation
/// composes the package path entries in order.
// Java: com/google/devtools/build/lib/pkgcache/PathPackageLocator.java:50 PathPackageLocator
#[derive(Clone)]
pub struct PathPackageLocator {
    path_entries: Vec<Root>,
    // Java marks it `transient` (not serialized); equals and hashCode still include it.
    output_base: Option<Path>,
    build_files_by_priority: Vec<BuildFileName>,
}

impl PathPackageLocator {
    // Java: com/google/devtools/build/lib/pkgcache/PathPackageLocator.java:62 PathPackageLocator.<init>
    pub fn new(
        output_base: Option<Path>,
        path_entries: Vec<Root>,
        build_files_by_priority: Vec<BuildFileName>,
    ) -> PathPackageLocator {
        PathPackageLocator {
            path_entries,
            output_base,
            build_files_by_priority,
        }
    }

    // Java: com/google/devtools/build/lib/pkgcache/PathPackageLocator.java:136 PathPackageLocator.maybeReplaceWorkspaceInString
    pub fn maybe_replace_workspace_in_string(
        path_element: &str,
        workspace: &PathFragment,
    ) -> String {
        path_element.replace(WORKSPACE_WILDCARD, workspace.get_path_string())
    }

    /// A factory of PathPackageLocators from a list of path elements. Elements may contain
    /// "%workspace%", indicating the workspace.
    ///
    /// `path_elements`: each element must be an absolute path, relative path, or some string
    /// "%workspace%" + relative, where relative is itself a relative path. The special symbol
    /// "%workspace%" means to interpret the path relative to the nearest enclosing workspace.
    /// Relative paths are interpreted relative to the client's working directory, which may be
    /// below the workspace.
    // Java: com/google/devtools/build/lib/pkgcache/PathPackageLocator.java:159 PathPackageLocator.create
    // Java: com/google/devtools/build/lib/pkgcache/PathPackageLocator.java:191 PathPackageLocator.createInternal
    pub fn create(
        output_base: Option<Path>,
        path_elements: &[String],
        event_handler: &dyn blaze_events::EventHandler,
        workspace: &PathFragment,
        client_working_directory: &Path,
        build_files_by_priority: Vec<BuildFileName>,
    ) -> PathPackageLocator {
        let mut resolved_paths: Vec<Root> = Vec::new();

        for path_element in path_elements {
            // Replace "%workspace%" with the path of the enclosing workspace directory.
            let path_element = Self::maybe_replace_workspace_in_string(path_element, workspace);

            let path_element_fragment = PathFragment::create(&path_element);

            // If the path string started with "%workspace%" or "/", it is already absolute, so
            // the following line returns a path pointing to pathElementFragment.
            let root_path = client_working_directory.get_relative(&path_element_fragment);

            if !path_element_fragment.is_absolute()
                && client_working_directory.as_fragment() != workspace
            {
                event_handler.handle(&blaze_events::Event::warn(format!(
                    "The package path element '{}' will be taken relative to your working \
                     directory. You may have intended to have the path taken relative to your \
                     workspace directory. If so, please usethe '{}' wildcard.",
                    path_element_fragment, WORKSPACE_WILDCARD
                )));
            }

            if root_path.exists() {
                resolved_paths.push(Root::from_path(root_path));
            }
        }

        PathPackageLocator::new(output_base, resolved_paths, build_files_by_priority)
    }

    /// Returns an immutable ordered list of the directories on the package path.
    // Java: com/google/devtools/build/lib/pkgcache/PathPackageLocator.java:127 PathPackageLocator.getPathEntries
    pub fn get_path_entries(&self) -> &[Root] {
        &self.path_entries
    }

    // Java: com/google/devtools/build/lib/pkgcache/PathPackageLocator.java:314 PathPackageLocator.getOutputBase
    pub fn get_output_base(&self) -> Option<&Path> {
        self.output_base.as_ref()
    }

    /// PORT: Java keeps this private.
    pub fn build_files_by_priority(&self) -> &[BuildFileName] {
        &self.build_files_by_priority
    }

    /// Returns the path to the build file for this package, or `None` if not found.
    ///
    /// If the same package exists beneath multiple package path entries, the first path that
    /// matches always wins.
    // PORT: the syscall cache is `SyscallCache.NO_CACHE` (a plain stat).
    // Java: com/google/devtools/build/lib/pkgcache/PathPackageLocator.java:81 PathPackageLocator.getPackageBuildFileNullable
    pub fn get_package_build_file_nullable(
        &self,
        package_identifier: &PackageIdentifier,
    ) -> Option<Path> {
        if package_identifier.get_repository().is_main() {
            for build_file_name in &self.build_files_by_priority {
                let build_file_path = self.get_file_path(
                    &package_identifier
                        .get_package_fragment()
                        .get_relative(build_file_name.get_filename_fragment()),
                );
                if build_file_path.is_some() {
                    return build_file_path;
                }
            }
        } else {
            let Some(output_base) = &self.output_base else {
                panic!(
                    "External package '{}' needs to be loaded but this PathPackageLocator instance \
                     does not support external packages",
                    package_identifier
                );
            };
            // This works only to some degree, because it relies on the presence of the repository
            // under $OUTPUT_BASE/external, which is created by the appropriate
            // RepositoryDirectoryValue. This is true for the invocation in GlobCache, but not for
            // the locator.getBuildFileForPackage() invocation in Parser#include().
            for build_file_name in &self.build_files_by_priority {
                let build_file = output_base
                    .get_relative(&EXTERNAL_REPOSITORY_LOCATION)
                    .get_relative_str(package_identifier.get_repository().get_name())
                    .get_relative(package_identifier.get_source_root())
                    .get_relative(build_file_name.get_filename_fragment());
                match build_file.stat_if_found(Symlinks::Follow) {
                    Ok(Some(stat)) if stat.is_file() => return Some(build_file),
                    Ok(_) => {}
                    Err(_) => return None,
                }
            }
        }
        None
    }

    // Java: com/google/devtools/build/lib/pkgcache/PathPackageLocator.java:275 PathPackageLocator.getFilePath
    fn get_file_path(&self, suffix: &PathFragment) -> Option<Path> {
        for path_entry in &self.path_entries {
            let build_file = path_entry.get_relative(suffix);
            // Java: SyscallCache.statusToDirentType (NO_CACHE skips getType).
            let type_ = match build_file.stat_if_found(Symlinks::Follow) {
                // Treat IOException as a missing file.
                Ok(Some(stat)) => Some(stat.get_dirent_type()),
                Ok(None) | Err(_) => None,
            };
            if type_ == Some(DirentType::File) || type_ == Some(DirentType::Unknown) {
                return Some(build_file);
            }
        }
        None
    }
}

// Java: com/google/devtools/build/lib/pkgcache/PathPackageLocator.java:132 PathPackageLocator.toString
impl fmt::Display for PathPackageLocator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PathPackageLocator[")?;
        for (i, e) in self.path_entries.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{}", e)?;
        }
        f.write_str("]")
    }
}

impl fmt::Debug for PathPackageLocator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

// Java: com/google/devtools/build/lib/pkgcache/PathPackageLocator.java:302 PathPackageLocator.equals
impl PartialEq for PathPackageLocator {
    fn eq(&self, other: &Self) -> bool {
        self.path_entries == other.path_entries && self.output_base == other.output_base
    }
}

impl Eq for PathPackageLocator {}

// Java: com/google/devtools/build/lib/pkgcache/PathPackageLocator.java:297 PathPackageLocator.hashCode
impl Hash for PathPackageLocator {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.path_entries.hash(state);
        self.output_base.hash(state);
    }
}
