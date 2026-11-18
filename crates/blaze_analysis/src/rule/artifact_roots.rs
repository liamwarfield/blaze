//! Conversion between the two ports of `actions/ArtifactRoot.java`: the output-directory roots
//! `blaze_build_config` creates for configurations (`output_directories::ArtifactRoot`, derived
//! roots only) and the full `blaze_actions::ArtifactRoot` that artifacts carry.
//!
//! Java has one class; `BuildConfigurationValue.getBinDirectory(...)` returns the same object
//! artifacts use. Both Rust types compare structurally, so converting on each call is equivalent.

use blaze_actions::actions::artifact_root::{ArtifactRoot, RootType};
use blaze_build_config::output_directories::{
    ArtifactRoot as ConfigArtifactRoot, RootType as ConfigRootType,
};

/// The `blaze_actions` root type of a configuration root type (same Java enum constants).
fn to_actions_root_type(t: ConfigRootType) -> RootType {
    match t {
        ConfigRootType::OUTPUT => RootType::OUTPUT,
        ConfigRootType::SIBLING_MAIN_OUTPUT => RootType::SIBLING_MAIN_OUTPUT,
        ConfigRootType::SIBLING_EXTERNAL_OUTPUT => RootType::SIBLING_EXTERNAL_OUTPUT,
    }
}

/// Converts a configuration's derived root to the `blaze_actions` root: the same
/// `ArtifactRoot.asDerivedRoot(execRoot, rootType, execPath)`.
// Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:135 ArtifactRoot.asDerivedRoot
pub fn to_actions_root(root: &ConfigArtifactRoot) -> ArtifactRoot {
    let exec_path = root.get_exec_path().clone();
    // The root path is `execRoot.getRelative(execPath)`: strip the exec path's segments.
    let mut exec_root = root.get_root_path().clone();
    for _ in 0..exec_path.segment_count() {
        exec_root = exec_root
            .get_parent_directory()
            .expect("derived root path is below its exec root");
    }
    ArtifactRoot::as_derived_root_fragment(
        &exec_root,
        to_actions_root_type(root.get_root_type()),
        exec_path,
    )
}

/// Alias of [`to_actions_root`] (the name the Starlark layer uses).
pub fn to_actions_artifact_root(root: &ConfigArtifactRoot) -> ArtifactRoot {
    to_actions_root(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_bin_root() {
        let fs = blaze_vfs::UnixFileSystem::create(blaze_vfs::DigestHashFunction::Sha256);
        let exec_root = blaze_vfs::Path::create_from_str("/execroot/_main", &fs);
        let config_root = ConfigArtifactRoot::as_derived_root(
            &exec_root,
            ConfigRootType::OUTPUT,
            &["bazel-out", "k8-fastbuild", "bin"],
        );
        let root = to_actions_root(&config_root);
        assert_eq!(root.get_exec_path_string(), "bazel-out/k8-fastbuild/bin");
        assert_eq!(root.get_root_type(), RootType::OUTPUT);
        assert_eq!(
            root.get_root().as_path().unwrap().get_path_string(),
            "/execroot/_main/bazel-out/k8-fastbuild/bin"
        );
    }
}
