//! Port of `lib/remote/common/RemotePathResolver.java`.

use std::sync::Arc;

use blaze_exec::actions::{ActionInput, InputMapping, PathMapper};
use blaze_exec::exec::SpawnExecutionContext;
use blaze_vfs::{Path, PathFragment};

/// A `RemotePathResolver` is used to resolve input/output paths for remote execution from Bazel's
/// internal path, or vice versa.
// Java: com/google/devtools/build/lib/remote/common/RemotePathResolver.java:36 RemotePathResolver
pub trait RemotePathResolver: Send + Sync {
    /// Returns the `workingDirectory` for a remote action.
    fn get_working_directory(&self) -> PathFragment;

    /// Returns a `SortedMap` which maps from input paths for remote action to `ActionInput`.
    // Java: com/google/devtools/build/lib/remote/common/RemotePathResolver.java:45 RemotePathResolver.getInputMapping
    fn get_input_mapping(
        &self,
        context: &dyn SpawnExecutionContext,
        will_access_repeatedly: bool,
    ) -> Arc<InputMapping> {
        context.get_input_mapping(&self.get_working_directory(), will_access_repeatedly)
    }

    /// Resolves the output path relative to input root for the given `Path`.
    fn local_path_to_output_path(&self, path: &Path) -> String;

    /// Resolves the output path relative to input root for the given exec path
    /// (`localPathToOutputPath(PathFragment execPath)`).
    fn local_exec_path_to_output_path(&self, exec_path: &PathFragment) -> String;

    /// Resolves the output path relative to input root for the `ActionInput`.
    // Java: com/google/devtools/build/lib/remote/common/RemotePathResolver.java:64 RemotePathResolver.localPathToOutputPath
    fn local_action_input_to_output_path(&self, action_input: &dyn ActionInput) -> String {
        self.local_exec_path_to_output_path(action_input.get_exec_path())
    }

    /// Resolves the local `Path` of an output file.
    fn output_path_to_local_path(&self, output_path: &str) -> Path;

    /// Resolves the exec path of a local path.
    fn local_path_to_exec_path(&self, local_path: &PathFragment) -> PathFragment;
}

/// Creates the default `RemotePathResolver`.
// Java: com/google/devtools/build/lib/remote/common/RemotePathResolver.java:79 RemotePathResolver.createDefault
pub fn create_default(exec_root: Path) -> Arc<dyn RemotePathResolver> {
    Arc::new(DefaultRemotePathResolver::new(exec_root))
}

/// The default `RemotePathResolver` which use `execRoot` as input root and do NOT set
/// `workingDirectory` for remote actions.
// Java: com/google/devtools/build/lib/remote/common/RemotePathResolver.java:87 RemotePathResolver.DefaultRemotePathResolver
#[derive(Debug, Clone)]
pub struct DefaultRemotePathResolver {
    exec_root: Path,
}

impl DefaultRemotePathResolver {
    /// `new DefaultRemotePathResolver(execRoot)`.
    pub fn new(exec_root: Path) -> DefaultRemotePathResolver {
        DefaultRemotePathResolver { exec_root }
    }
}

impl RemotePathResolver for DefaultRemotePathResolver {
    fn get_working_directory(&self) -> PathFragment {
        PathFragment::empty_fragment()
    }

    fn local_path_to_output_path(&self, path: &Path) -> String {
        path.relative_to(&self.exec_root).get_path_string().to_string()
    }

    fn local_exec_path_to_output_path(&self, exec_path: &PathFragment) -> String {
        exec_path.get_path_string().to_string()
    }

    fn output_path_to_local_path(&self, output_path: &str) -> Path {
        self.exec_root.get_relative_str(output_path)
    }

    fn local_path_to_exec_path(&self, local_path: &PathFragment) -> PathFragment {
        local_path.relative_to(self.exec_root.as_fragment())
    }
}

/// A `RemotePathResolver` used when `--experimental_sibling_repository_layout` is set. Use parent
/// directory of `execRoot` and set `workingDirectory` to the base name of `execRoot`.
///
/// The paths of outputs are relative to `workingDirectory` if
/// `--incompatible_remote_paths_relative_to_input_root` is not set, otherwise, relative to input
/// root.
// Java: com/google/devtools/build/lib/remote/common/RemotePathResolver.java:128 RemotePathResolver.SiblingRepositoryLayoutResolver
#[derive(Debug, Clone)]
pub struct SiblingRepositoryLayoutResolver {
    exec_root: Path,
    working_directory: PathFragment,
}

impl SiblingRepositoryLayoutResolver {
    /// `new SiblingRepositoryLayoutResolver(execRoot)`.
    // Java: com/google/devtools/build/lib/remote/common/RemotePathResolver.java:133 SiblingRepositoryLayoutResolver.<init>
    pub fn new(exec_root: Path) -> SiblingRepositoryLayoutResolver {
        // The "root directory" of the action from the point of view of RBE is the parent
        // directory of the execroot locally. This is so that paths of artifacts in external
        // repositories don't start with an uplevel reference.
        let working_directory = PathFragment::create(exec_root.get_base_name());
        SiblingRepositoryLayoutResolver {
            exec_root,
            working_directory,
        }
    }

    fn get_base(&self) -> &Path {
        &self.exec_root
    }
}

impl RemotePathResolver for SiblingRepositoryLayoutResolver {
    fn get_working_directory(&self) -> PathFragment {
        self.working_directory.clone()
    }

    fn local_path_to_output_path(&self, path: &Path) -> String {
        path.relative_to(self.get_base()).get_path_string().to_string()
    }

    fn local_exec_path_to_output_path(&self, exec_path: &PathFragment) -> String {
        self.local_path_to_output_path(&self.exec_root.get_relative(exec_path))
    }

    fn output_path_to_local_path(&self, output_path: &str) -> Path {
        self.get_base().get_relative_str(output_path)
    }

    fn local_path_to_exec_path(&self, local_path: &PathFragment) -> PathFragment {
        local_path.relative_to(self.get_base().as_fragment())
    }
}

/// Adapts a given base `RemotePathResolver` to also apply a `PathMapper` to map (and inverse map)
/// paths.
// Java: com/google/devtools/build/lib/remote/common/RemotePathResolver.java:174 RemotePathResolver.createMapped
pub fn create_mapped(
    base: Arc<dyn RemotePathResolver>,
    _exec_root: &Path,
    path_mapper: &PathMapper,
) -> Arc<dyn RemotePathResolver> {
    if path_mapper.is_noop() {
        return base;
    }
    // PORT: blaze_exec's PathMapper has only the NOOP mapper.
    blaze_util::unported!("RemotePathResolver.createMapped (path mapping)")
}
