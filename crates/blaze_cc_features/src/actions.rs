//! Minimal stand-ins for the `lib/actions` types the C++ core touches (`Artifact`, `PathMapper`,
//! `InputMetadataProvider`, `ArtifactResolver`, `ParameterFile.ParameterFileType`, the
//! `ActionExecutionException` / `CommandLineExpansionException` errors) and for `util/OS`.
//!
//! PORT: no Blaze crate ports `lib/actions` yet. These are traits so that the actions crate can
//! implement them for its real types later; only the members this crate calls are declared, with
//! Java's names and semantics.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_vfs::PathFragment;

/// The parts of `com.google.devtools.build.lib.actions.Artifact` used here.
// Java: com/google/devtools/build/lib/actions/Artifact.java:119 Artifact
pub trait Artifact: Send + Sync + fmt::Debug {
    /// Java: `Artifact.getExecPath`.
    fn get_exec_path(&self) -> PathFragment;
    /// Java: `Artifact.getExecPathString`.
    fn get_exec_path_string(&self) -> String {
        self.get_exec_path().get_path_string().to_string()
    }
    /// Java: `Artifact.isTreeArtifact`.
    fn is_tree_artifact(&self) -> bool;
    /// Java: `Artifact.isSourceArtifact`.
    fn is_source_artifact(&self) -> bool;
    /// Java: `Artifact.prettyPrint`.
    fn pretty_print(&self) -> String;
    /// Java: `Artifact.equals`.
    fn artifact_equals(&self, other: &dyn Artifact) -> bool;
    /// Java: `Artifact.hashCode` (any hash consistent with [`Artifact::artifact_equals`]).
    fn artifact_hash(&self) -> u64;
}

/// Shared handle to an [`Artifact`] with Java's `equals`/`hashCode`.
#[derive(Clone, Debug)]
pub struct ArtifactRef(pub Arc<dyn Artifact>);

impl ArtifactRef {
    pub fn new<A: Artifact + 'static>(artifact: A) -> ArtifactRef {
        ArtifactRef(Arc::new(artifact))
    }
}

impl std::ops::Deref for ArtifactRef {
    type Target = dyn Artifact;
    fn deref(&self) -> &(dyn Artifact + 'static) {
        &*self.0
    }
}

impl PartialEq for ArtifactRef {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.artifact_equals(&*other.0)
    }
}

impl Eq for ArtifactRef {}

impl Hash for ArtifactRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.0.artifact_hash());
    }
}

/// Port of `com.google.devtools.build.lib.actions.PathMapper` (the members used here).
// Java: com/google/devtools/build/lib/actions/PathMapper.java:42 PathMapper
pub trait PathMapper: Send + Sync {
    // Java: com/google/devtools/build/lib/actions/PathMapper.java:102 PathMapper.map
    fn map(&self, exec_path: &PathFragment) -> PathFragment;

    // Java: com/google/devtools/build/lib/actions/PathMapper.java:105 PathMapper.getMappedExecPathString
    fn get_mapped_exec_path_string(&self, artifact: &dyn Artifact) -> String {
        self.map(&artifact.get_exec_path())
            .get_path_string()
            .to_string()
    }

    // Java: com/google/devtools/build/lib/actions/PathMapper.java:150 PathMapper.mapHeuristically
    fn map_heuristically(&self, arg: &str) -> String {
        arg.to_string()
    }

    // Java: com/google/devtools/build/lib/actions/PathMapper.java:185 PathMapper.isNoop
    // PORT: Java compares with the NOOP singleton; only [`NoopPathMapper`] overrides this.
    fn is_noop(&self) -> bool {
        false
    }
}

/// Java: `PathMapper.NOOP`.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopPathMapper;

// Java: com/google/devtools/build/lib/actions/PathMapper.java:201 PathMapper.NOOP
impl PathMapper for NoopPathMapper {
    fn map(&self, exec_path: &PathFragment) -> PathFragment {
        exec_path.clone()
    }

    fn get_mapped_exec_path_string(&self, artifact: &dyn Artifact) -> String {
        artifact.get_exec_path_string()
    }

    fn is_noop(&self) -> bool {
        true
    }
}

/// Java: `PathMapper.NOOP`.
pub static NOOP: NoopPathMapper = NoopPathMapper;

/// The member of `com.google.devtools.build.lib.actions.InputMetadataProvider` used here.
// Java: com/google/devtools/build/lib/actions/InputMetadataProvider.java:33 InputMetadataProvider
pub trait InputMetadataProvider: Send + Sync {
    /// Java: `getTreeMetadata(artifact)`, reduced to `TreeArtifactValue.getChildren()`; `None`
    /// where Java returns null.
    fn get_tree_metadata(&self, artifact: &ArtifactRef) -> Option<Vec<ArtifactRef>>;
}

/// The members of `com.google.devtools.build.lib.actions.ArtifactResolver` used by
/// `HeaderDiscovery`.
// Java: com/google/devtools/build/lib/actions/ArtifactResolver.java:30 ArtifactResolver
pub trait ArtifactResolver {
    fn resolve_source_artifact(
        &self,
        exec_path: &PathFragment,
        repository_name: &blaze_cmdline::RepositoryName,
    ) -> Option<ArtifactRef>;

    fn resolve_source_artifacts_ascii_case_insensitively(
        &self,
        exec_path: &PathFragment,
        repository_name: &blaze_cmdline::RepositoryName,
    ) -> Vec<ArtifactRef>;

    fn is_derived_artifact(&self, exec_path: &PathFragment) -> bool;
}

/// Port of `com.google.devtools.build.lib.actions.ParameterFile.ParameterFileType`.
// Java: com/google/devtools/build/lib/actions/ParameterFile.java:37 ParameterFile.ParameterFileType
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParameterFileType {
    UNQUOTED,
    SHELL_QUOTED,
    GCC_QUOTED,
    WINDOWS,
}

impl ParameterFileType {
    /// Java: `ParameterFileType.valueOf(name)`; `None` where Java throws
    /// `IllegalArgumentException`.
    pub fn value_of(name: &str) -> Option<ParameterFileType> {
        match name {
            "UNQUOTED" => Some(ParameterFileType::UNQUOTED),
            "SHELL_QUOTED" => Some(ParameterFileType::SHELL_QUOTED),
            "GCC_QUOTED" => Some(ParameterFileType::GCC_QUOTED),
            "WINDOWS" => Some(ParameterFileType::WINDOWS),
            _ => None,
        }
    }
}

/// The fields of `com.google.devtools.build.lib.actions.ParamFileInfo` set by the C++ code.
// Java: com/google/devtools/build/lib/actions/ParamFileInfo.java:30 ParamFileInfo
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamFileInfo {
    pub file_type: ParameterFileType,
    pub flag_format_string: String,
    pub use_always: bool,
}

/// Port of `com.google.devtools.build.lib.actions.CommandLineExpansionException` (message only).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct CommandLineExpansionException {
    pub message: String,
}

/// `FailureDetails.CppCompile.Code` values raised by the ported code.
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CppCompileCode {
    UNDECLARED_INCLUSIONS,
    COMMAND_GENERATION_FAILURE,
}

/// Port of `com.google.devtools.build.lib.actions.ActionExecutionException` as thrown by the C++
/// include checks: the message, `catastrophe` and the `CppCompile` failure code.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ActionExecutionException {
    pub message: String,
    pub catastrophe: bool,
    pub code: CppCompileCode,
}

/// Port of `com.google.devtools.build.lib.util.OS`.
// Java: com/google/devtools/build/lib/util/OS.java:21 OS
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OS {
    DARWIN,
    FREEBSD,
    OPENBSD,
    LINUX,
    WINDOWS,
    UNKNOWN,
}

impl OS {
    /// Java: `OS.valueOf(name)`; `None` where Java throws `IllegalArgumentException`.
    pub fn value_of(name: &str) -> Option<OS> {
        match name {
            "DARWIN" => Some(OS::DARWIN),
            "FREEBSD" => Some(OS::FREEBSD),
            "OPENBSD" => Some(OS::OPENBSD),
            "LINUX" => Some(OS::LINUX),
            "WINDOWS" => Some(OS::WINDOWS),
            "UNKNOWN" => Some(OS::UNKNOWN),
            _ => None,
        }
    }
}

/// Java: `PathFragment.createForOs(path, os)`.
// Java: com/google/devtools/build/lib/vfs/PathFragment.java:129 PathFragment.createForOs
pub fn path_fragment_create_for_os(path: &str, os: OS) -> PathFragment {
    match os {
        OS::WINDOWS => blaze_util::unported!("PathFragment.createForOs(WINDOWS)"),
        // PORT: every non-Windows OS uses the Unix path policy, as in Java.
        _ => PathFragment::create(path),
    }
}
