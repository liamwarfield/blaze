//! Port of `src/main/cpp/workspace_layout.{h,cc}`.

use crate::util::file::{is_directory, path_exists};
use crate::util::path::{basename, dirname, is_root_directory, join_path};

// Java: src/main/cpp/workspace_layout.h:25 WorkspaceLayout
/// Provides methods to compute paths related to the workspace.
#[derive(Clone, Copy, Debug, Default)]
pub struct WorkspaceLayout;

impl WorkspaceLayout {
    // Java: src/main/cpp/workspace_layout.h:63 WorkspaceLayout::kWorkspacePrefix
    pub const K_WORKSPACE_PREFIX: &'static str = "%workspace%/";

    pub fn new() -> Self {
        WorkspaceLayout
    }

    // Java: src/main/cpp/workspace_layout.cc:33 WorkspaceLayout::InWorkspace
    /// Returns if workspace is a valid build workspace.
    pub fn in_workspace(&self, workspace: &str) -> bool {
        for boundary_file_name in ["MODULE.bazel", "REPO.bazel", "WORKSPACE.bazel", "WORKSPACE"] {
            let boundary_file_path = join_path(workspace, boundary_file_name);
            if path_exists(&boundary_file_path) && !is_directory(&boundary_file_path) {
                return true;
            }
        }
        false
    }

    // Java: src/main/cpp/workspace_layout.cc:44 WorkspaceLayout::GetWorkspace
    /// Given the working directory, returns the nearest enclosing directory with a workspace
    /// boundary file in it. If there is no such enclosing directory, returns "".
    pub fn get_workspace(&self, cwd: &str) -> String {
        assert!(!cwd.is_empty());
        let mut workspace = cwd.to_string();
        loop {
            if self.in_workspace(&workspace) {
                return workspace;
            }
            workspace = dirname(&workspace);
            if workspace.is_empty() || is_root_directory(&workspace) {
                break;
            }
        }
        String::new()
    }

    // Java: src/main/cpp/workspace_layout.cc:57 WorkspaceLayout::GetPrettyWorkspaceName
    /// e.g. A Bazel server process running in ~/src/myproject (where there's a
    /// ~/src/myproject/WORKSPACE file) will appear in ps(1) as "bazel(myproject)".
    pub fn get_pretty_workspace_name(&self, workspace: &str) -> String {
        basename(workspace)
    }

    // Java: src/main/cpp/workspace_layout.cc:64 WorkspaceLayout::GetWorkspaceRcPath
    pub fn get_workspace_rc_path(&self, workspace: &str, _startup_args: &[String]) -> String {
        join_path(workspace, "tools/bazel.rc")
    }

    // Java: src/main/cpp/workspace_layout.cc:73 WorkspaceLayout::ResolveWorkspaceRelativeRcFilePath
    /// Resolves a %workspace%-relative import into an actual path, or `None` if it could not be
    /// resolved into an existing file.
    pub fn resolve_workspace_relative_rc_file_path(
        &self,
        workspace: &str,
        import_path: &str,
    ) -> Option<String> {
        // Strip off the "%workspace%/" prefix and prepend the true workspace path.
        assert!(import_path.starts_with(Self::K_WORKSPACE_PREFIX));
        let resolved_path = join_path(workspace, &import_path[Self::K_WORKSPACE_PREFIX.len()..]);
        if path_exists(&resolved_path) {
            return Some(resolved_path);
        }
        None
    }
}
