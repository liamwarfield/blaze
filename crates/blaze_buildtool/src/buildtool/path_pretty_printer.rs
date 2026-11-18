//! Port of `buildtool/PathPrettyPrinter.java`.

use blaze_util::check_argument;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

/// Uses information about the convenience symlinks to print shorter paths for output artifacts.
///
/// Instances cannot be reused across builds - they must be used for the build associated with the
/// provided symlinks. If instances are reused the pretty path may be incorrect, for example if the
/// symlinks end up pointing somewhere new.
// Java: com/google/devtools/build/lib/buildtool/PathPrettyPrinter.java:28 PathPrettyPrinter
#[derive(Clone, Debug)]
pub struct PathPrettyPrinter {
    workspace_relative_to_pwd: PathFragment,
    symlink_prefix: String,
    resolved_symlinks: IndexMap<PathFragment, PathFragment>,
}

// Java: com/google/devtools/build/lib/buildtool/PathPrettyPrinter.java:29 PathPrettyPrinter.NO_CREATE_SYMLINKS_PREFIX
const NO_CREATE_SYMLINKS_PREFIX: &str = "/";

impl PathPrettyPrinter {
    /// Creates an instance.
    ///
    /// The provided workspace-relative working directory must be a relative path.
    // Java: com/google/devtools/build/lib/buildtool/PathPrettyPrinter.java:40 PathPrettyPrinter.<init>
    pub fn new(
        workspace_relative_working_directory: &PathFragment,
        symlink_prefix: &str,
        convenience_symlinks: IndexMap<PathFragment, PathFragment>,
    ) -> PathPrettyPrinter {
        check_argument!(
            !workspace_relative_working_directory.is_absolute(),
            "workspaceRelativeWorkingDirectory must be relative to the workspace"
        );
        PathPrettyPrinter {
            workspace_relative_to_pwd: PathFragment::create(
                &"../".repeat(workspace_relative_working_directory.segment_count()),
            ),
            symlink_prefix: symlink_prefix.to_string(),
            resolved_symlinks: convenience_symlinks,
        }
    }

    /// Returns a convenient path to the specified file, relativizing it and using convenience
    /// symlinks if possible. Otherwise, return the original path.
    // Java: com/google/devtools/build/lib/buildtool/PathPrettyPrinter.java:57 PathPrettyPrinter.getPrettyPath
    pub fn get_pretty_path(&self, file: &PathFragment) -> PathFragment {
        if NO_CREATE_SYMLINKS_PREFIX == self.symlink_prefix {
            return file.clone();
        }

        for (key, link_target) in &self.resolved_symlinks {
            let link_fragment = self.workspace_relative_to_pwd.get_relative(key);
            if file.starts_with(link_target) {
                return link_fragment.get_relative(&file.relative_to(link_target));
            }
        }

        file.clone()
    }
}
