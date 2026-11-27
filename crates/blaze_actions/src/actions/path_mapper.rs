//! Ports of `lib/actions/PathMapper.java` and `PathMapperConstants.java`.
//!
//! PORT: replaces the stand-in `PathMapper` of `blaze_cc_features::actions`; that crate should
//! switch to this trait (it has the same `map`, `get_mapped_exec_path_string`,
//! `map_heuristically` and `is_noop` members).

use std::fmt;

use blaze_collect::command_line_item::{DEFAULT, ExceptionlessMapFn};
use blaze_vfs::PathFragment;

use super::artifact::Artifact;
use super::artifact_root::ArtifactRoot;
use super::command_line::ArgChunk;

/// The output paths mode (`--experimental_output_paths`).
// Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:913 CoreOptions.OutputPathsMode
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OutputPathsMode {
    /// Use the production output path model.
    OFF,
    /// Strip the config prefix (i.e. `/x86-fastbuild/` from output paths for actions that are
    /// registered to support this feature.
    STRIP,
}

/// What `mapRoot` returns: the artifact's own root, or a mapped root.
// Java: com/google/devtools/build/lib/starlarkbuildapi/FileRootApi.java FileRootApi
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FileRoot {
    Artifact(ArtifactRoot),
    Mapped(MappedArtifactRoot),
}

impl FileRoot {
    // Java: com/google/devtools/build/lib/starlarkbuildapi/FileRootApi.java FileRootApi.getExecPathString
    pub fn get_exec_path_string(&self) -> String {
        match self {
            FileRoot::Artifact(r) => r.get_exec_path_string().to_string(),
            FileRoot::Mapped(r) => r.get_exec_path_string().to_string(),
        }
    }
}

/// A root for files that have been subject to path mapping.
// Java: com/google/devtools/build/lib/actions/PathMapper.java:232 PathMapper.MappedArtifactRoot
// (equals/hashCode derived over the mapped root's exec path)
// Java: com/google/devtools/build/lib/actions/PathMapper.java:247 PathMapper.MappedArtifactRoot.equals
// Java: com/google/devtools/build/lib/actions/PathMapper.java:259 PathMapper.MappedArtifactRoot.hashCode
// PORT: skipped, serialization only (immutability of a codec value): MappedArtifactRoot.isImmutable
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MappedArtifactRoot {
    mapped_root_exec_path: PathFragment,
}

impl MappedArtifactRoot {
    pub fn new(mapped_root_exec_path: PathFragment) -> MappedArtifactRoot {
        MappedArtifactRoot {
            mapped_root_exec_path,
        }
    }

    // Java: com/google/devtools/build/lib/actions/PathMapper.java:237 MappedArtifactRoot.getExecPathString
    pub fn get_exec_path_string(&self) -> &str {
        self.mapped_root_exec_path.get_path_string()
    }

    // Java: com/google/devtools/build/lib/actions/PathMapper.java:269 MappedArtifactRoot.repr
    pub fn repr(&self) -> &'static str {
        "<mapped root>"
    }
}

// Java: com/google/devtools/build/lib/actions/PathMapper.java:264 MappedArtifactRoot.toString
impl fmt::Display for MappedArtifactRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} [mapped]", self.mapped_root_exec_path)
    }
}

/// Support for mapping config parts of exec paths in an action's command line as well as when
/// staging its inputs and outputs for execution, with the general aim of making the action
/// cache-friendly across configurations.
// Java: com/google/devtools/build/lib/actions/PathMapper.java:42 PathMapper
pub trait PathMapper: Send + Sync {
    /// Returns the exec path with the path mapping applied.
    // Java: com/google/devtools/build/lib/actions/PathMapper.java:102 PathMapper.map
    fn map(&self, exec_path: &PathFragment) -> PathFragment;

    /// Returns the exec path of the input with the path mapping applied.
    // Java: com/google/devtools/build/lib/actions/PathMapper.java:105 PathMapper.getMappedExecPathString
    fn get_mapped_exec_path_string(&self, artifact: &Artifact) -> String {
        self.map(artifact.get_exec_path())
            .get_path_string()
            .to_string()
    }

    /// We don't yet have a Starlark API for mapping paths in command lines. Simple Starlark calls
    /// like `args.add(arg_name, file_path` are automatically handled. But calls that involve
    /// custom Starlark code require deeper API support that remains a TODO.
    ///
    /// This method allows implementations to hard-code support for specific command line
    /// entries for specific Starlark actions.
    // Java: com/google/devtools/build/lib/actions/PathMapper.java:128 PathMapper.mapCustomStarlarkArgs
    fn map_custom_starlark_args<'a>(
        &self,
        chunk: Box<dyn ArgChunk + 'a>,
    ) -> Box<dyn ArgChunk + 'a> {
        chunk
    }

    // Java: com/google/devtools/build/lib/actions/PathMapper.java:116 PathMapper.computeExecPathLengthDiff
    fn compute_exec_path_length_diff(&self, artifact: &Artifact) -> i32 {
        // Java `length()`: chars, one per byte of an internal string.
        artifact.get_exec_path_string().chars().count() as i32
            - self.get_mapped_exec_path_string(artifact).chars().count() as i32
    }

    /// Returns a `MapFn` that can be applied to an object to obtain its mapped string
    /// representation, given the flag that precedes it (if any).
    // Java: com/google/devtools/build/lib/actions/PathMapper.java:145 PathMapper.getMapFn
    // PORT: Java returns an `ExceptionlessMapFn<Object>`; command lines here expand artifacts and
    // strings, so the map fn is over artifacts.
    fn get_map_fn(
        &self,
        _previous_flag: Option<&str>,
    ) -> &'static dyn ExceptionlessMapFn<Artifact> {
        &DEFAULT
    }

    /// Heuristically maps all path-like strings in the given argument.
    // Java: com/google/devtools/build/lib/actions/PathMapper.java:150 PathMapper.mapHeuristically
    fn map_heuristically(&self, arg: &str) -> String {
        arg.to_string()
    }

    /// Returns the root of the given artifact with the path mapping applied.
    // Java: com/google/devtools/build/lib/actions/PathMapper.java:159 PathMapper.mapRoot
    fn map_root(&self, artifact: &Artifact) -> FileRoot {
        let root = artifact.get_root();
        if root.is_source_root() {
            // Java: PathMapperConstants.mappedSourceRoots (a cache of new MappedArtifactRoot).
            return FileRoot::Mapped(MappedArtifactRoot::new(root.get_exec_path().clone()));
        }
        let exec_path = artifact.get_exec_path();
        let mapped_exec_path = self.map(exec_path);
        let root_relative_segment_count =
            exec_path.segment_count() - root.get_exec_path().segment_count();
        let mapped_root_exec_path = mapped_exec_path.sub_fragment(
            0,
            mapped_exec_path.segment_count() - root_relative_segment_count,
        );
        FileRoot::Mapped(MappedArtifactRoot::new(mapped_root_exec_path))
    }

    /// Returns true if the mapper is known to map all paths identically.
    // Java: com/google/devtools/build/lib/actions/PathMapper.java:185 PathMapper.isNoop
    // PORT: Java compares with the NOOP singleton; only [`NoopPathMapper`] overrides this.
    fn is_noop(&self) -> bool {
        false
    }

    /// Returns an opaque object whose equality class should encompass all information that can
    /// influence the behavior of `map`.
    // Java: com/google/devtools/build/lib/actions/PathMapper.java:196 PathMapper.cacheKey
    // PORT: Java returns `getClass()`; the Rust type name stands in for the class.
    fn cache_key(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

/// `PathMapper.NOOP`: a `PathMapper` that doesn't map any paths.
// Java: com/google/devtools/build/lib/actions/PathMapper.java:201 PathMapper.NOOP
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopPathMapper;

impl PathMapper for NoopPathMapper {
    fn map(&self, exec_path: &PathFragment) -> PathFragment {
        exec_path.clone()
    }

    fn get_mapped_exec_path_string(&self, artifact: &Artifact) -> String {
        artifact.get_exec_path_string().to_string()
    }

    fn compute_exec_path_length_diff(&self, _artifact: &Artifact) -> i32 {
        0
    }

    fn map_root(&self, artifact: &Artifact) -> FileRoot {
        FileRoot::Artifact(artifact.get_root().clone())
    }

    fn is_noop(&self) -> bool {
        true
    }
}

/// `PathMapper.NOOP`.
pub static NOOP: NoopPathMapper = NoopPathMapper;

/// `PathMapperConstants.FOR_FINGERPRINTING`: a path mapper that inserts `pm-` before the config
/// segment of output paths, used to fingerprint path-mapped actions.
// PORT: skipped, PathMapperConstants.PathMapperConstants: the private constructor of a static utility class.
// Java: com/google/devtools/build/lib/actions/PathMapperConstants.java:63 PathMapperConstants.FOR_FINGERPRINTING
#[derive(Clone, Copy, Debug, Default)]
pub struct ForFingerprintingPathMapper;

impl PathMapper for ForFingerprintingPathMapper {
    fn map(&self, exec_path: &PathFragment) -> PathFragment {
        if !exec_path.starts_with(&PathFragment::create("bazel-out"))
            && !exec_path.starts_with(&PathFragment::create("blaze-out"))
        {
            return exec_path.clone();
        }
        let exec_path_string = exec_path.get_path_string();
        let start_of_config_segment = match exec_path_string.find('/') {
            None => return exec_path.clone(),
            Some(i) => i + 1,
        };
        PathFragment::create_already_normalized(&format!(
            "{}pm-{}",
            &exec_path_string[..start_of_config_segment],
            &exec_path_string[start_of_config_segment..]
        ))
    }
}

pub static FOR_FINGERPRINTING: ForFingerprintingPathMapper = ForFingerprintingPathMapper;

/// Returns the `PathMapper` to be used to compute the action key of an action with the given
/// effective output paths mode.
// Java: com/google/devtools/build/lib/actions/PathMapper.java:87 PathMapper.forActionKey
pub fn for_action_key(effective_output_paths_mode: OutputPathsMode) -> &'static dyn PathMapper {
    if effective_output_paths_mode == OutputPathsMode::OFF {
        &NOOP
    } else {
        &FOR_FINGERPRINTING
    }
}
