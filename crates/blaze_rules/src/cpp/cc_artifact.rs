//! Adapters between `blaze_actions` (the real `Artifact`, `PathMapper`, `InputMetadataProvider`,
//! `ParamFileInfo`) and the stand-in traits of `blaze_cc_features::actions`, which the C++
//! feature engine was ported against before `lib/actions` existed.
//!
//! PORT: Java has one `Artifact` class; the two Rust views are bridged here without changing
//! behaviour: every method delegates to the real artifact.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::actions::param_file_info::ParamFileInfo;
use blaze_actions::actions::parameter_file::ParameterFileType;
use blaze_actions::actions::path_mapper::PathMapper;
use blaze_cc_features::actions as cc;
use blaze_vfs::PathFragment;

/// A real artifact seen through `blaze_cc_features::actions::Artifact`.
#[derive(Debug, Clone)]
pub struct CcArtifact {
    artifact: Artifact,
    /// Java `hashCode()` of the artifact (any hash consistent with `equals`).
    hash: u64,
}

impl CcArtifact {
    pub fn new(artifact: Artifact) -> CcArtifact {
        let mut h = DefaultHasher::new();
        artifact.hash(&mut h);
        CcArtifact {
            artifact,
            hash: h.finish(),
        }
    }

    pub fn artifact(&self) -> &Artifact {
        &self.artifact
    }
}

impl cc::Artifact for CcArtifact {
    fn get_exec_path(&self) -> PathFragment {
        self.artifact.get_exec_path().clone()
    }

    fn get_exec_path_string(&self) -> String {
        self.artifact.get_exec_path_string().to_owned()
    }

    fn is_tree_artifact(&self) -> bool {
        self.artifact.is_tree_artifact()
    }

    fn is_source_artifact(&self) -> bool {
        self.artifact.is_source_artifact()
    }

    fn pretty_print(&self) -> String {
        self.artifact.pretty_print()
    }

    // PORT: the trait gives no downcast; two adapters are equal when the real artifacts' hashes
    // (std `Hash` of `blaze_actions::Artifact`, consistent with its `Eq`) and exec paths agree.
    fn artifact_equals(&self, other: &dyn cc::Artifact) -> bool {
        other.artifact_hash() == self.hash
            && other.get_exec_path() == *self.artifact.get_exec_path()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn artifact_hash(&self) -> u64 {
        self.hash
    }
}

/// Wraps a real artifact as a `blaze_cc_features` artifact reference.
pub fn to_cc_artifact(artifact: &Artifact) -> cc::ArtifactRef {
    cc::ArtifactRef(Arc::new(CcArtifact::new(artifact.clone())))
}

/// A `blaze_actions::PathMapper` seen as the `blaze_cc_features` path mapper.
pub struct CcPathMapper<'a>(pub &'a dyn PathMapper);

impl cc::PathMapper for CcPathMapper<'_> {
    fn map(&self, exec_path: &PathFragment) -> PathFragment {
        self.0.map(exec_path)
    }

    fn map_heuristically(&self, arg: &str) -> String {
        self.0.map_heuristically(arg)
    }

    fn is_noop(&self) -> bool {
        self.0.is_noop()
    }
}

/// A `blaze_actions::InputMetadataProvider` seen as the `blaze_cc_features` one.
pub struct CcInputMetadataProvider<'a>(pub &'a dyn blaze_actions::InputMetadataProvider);

impl cc::InputMetadataProvider for CcInputMetadataProvider<'_> {
    // Java: com/google/devtools/build/lib/actions/InputMetadataProvider.java:56 InputMetadataProvider.getTreeMetadata
    fn get_tree_metadata(&self, artifact: &cc::ArtifactRef) -> Option<Vec<cc::ArtifactRef>> {
        let artifact = artifact
            .0
            .as_any()
            .and_then(|a| a.downcast_ref::<CcArtifact>())
            .unwrap_or_else(|| panic!("ClassCastException: {:?} is not an Artifact", artifact.0))
            .artifact();
        // TreeArtifactValue.getChildren()
        let children = self.0.get_tree_children(artifact)?;
        Some(children.iter().map(to_cc_artifact).collect())
    }
}

/// `ParameterFileType` of `blaze_cc_features` as the `blaze_actions` enum (same Java enum).
pub fn to_actions_parameter_file_type(t: cc::ParameterFileType) -> ParameterFileType {
    match t {
        cc::ParameterFileType::UNQUOTED => ParameterFileType::UNQUOTED,
        cc::ParameterFileType::SHELL_QUOTED => ParameterFileType::SHELL_QUOTED,
        cc::ParameterFileType::GCC_QUOTED => ParameterFileType::GCC_QUOTED,
        cc::ParameterFileType::WINDOWS => ParameterFileType::WINDOWS,
    }
}

/// The `blaze_cc_features` `ParamFileInfo` (what `LinkCommandLine.getParamFileInfo` sets) as the
/// `blaze_actions` value: `ParamFileInfo.builder(type).setFlagFormatString(f).setUseAlways(a)`.
// Java: com/google/devtools/build/lib/rules/cpp/LinkCommandLine.java:128 LinkCommandLine.getParamFileInfo
pub fn to_actions_param_file_info(info: &cc::ParamFileInfo) -> ParamFileInfo {
    ParamFileInfo::builder(to_actions_parameter_file_type(info.file_type))
        .set_flag_format_string(info.flag_format_string.clone())
        .set_use_always(info.use_always)
        .build()
}
