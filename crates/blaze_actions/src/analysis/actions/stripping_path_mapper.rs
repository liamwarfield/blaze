//! Port of `analysis/actions/StrippingPathMapper.java`: the main logic for experimental
//! config-stripped execution paths (<https://github.com/bazelbuild/bazel/issues/6526>).
//!
//! The actions executors run look like: `tool_pkg/mytool src/source.file
//! bazel-out/x86-opt/pkg/gen.file -o bazel-out/x86-opt/pkg/myout`. The "x86-opt" part is a path's
//! "configuration prefix". Config-stripping (`--experimental_output_paths=strip`) strips it from
//! qualifying actions before running them: an action qualifies if it uses
//! [`path_mappers::create`](super::path_mappers::create) and its mnemonic is in
//! `PathMappers.SUPPORTED_MNEMONICS` (or it has the `supports-path-mapping` execution
//! requirement); it must then pass the [`PathMapper`] to all structured command line
//! constructions. A supporting executor strips paths from qualifying actions' inputs and outputs
//! before staging for execution.

use std::collections::HashMap;

use blaze_collect::command_line_item::{ExceptionlessMapFn, MapFn, MapFnError};
use blaze_util::unported;
use blaze_vfs::PathFragment;

use crate::actions::action::{Action, InputMetadataProvider};
use crate::actions::artifact::Artifact;
use crate::actions::artifact_root::ArtifactRoot;
use crate::actions::command_line::ArgChunk;
use crate::actions::path_mapper::{FileRoot, MappedArtifactRoot, PathMapper};

// Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:73 StrippingPathMapper.GUID
pub const GUID: &str = "8eb2ad5a-85d4-435b-858f-5c192e91997d";

// Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:75 StrippingPathMapper.FIXED_CONFIG_SEGMENT
const FIXED_CONFIG_SEGMENT: &str = "cfg";

// Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:72 StrippingPathMapper
#[derive(Debug)]
pub struct StrippingPathMapper {
    output_root: PathFragment,
    mnemonic: String,
    is_starlark_action: bool,
    is_java_action: bool,
    arg_stripper: StringStripper,
    output_artifact_root: ArtifactRoot,
    stripped_output_artifact_root: MappedArtifactRoot,
}

/// `structuredArgStripper`: strips the configuration prefix from a `String`; any other object
/// expands as `CommandLineItem.expandToCommandLine(object)`.
///
/// PORT: the Rust `PathMapper::get_map_fn` is typed over artifacts (an artifact expands to its
/// unmapped exec path, as Java's `expandToCommandLine`); for a string value
/// `custom_command_line` recognises this map fn and calls the path mapper's `map_heuristically`,
/// which is Java's `argStripper.strip`.
// Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:92 StrippingPathMapper.<init>
pub struct StructuredArgStripper;

/// The [`StructuredArgStripper`] instance.
pub static STRUCTURED_ARG_STRIPPER: StructuredArgStripper = StructuredArgStripper;

impl MapFn<Artifact> for StructuredArgStripper {
    fn expand_to_command_line(
        &self,
        object: &Artifact,
        args: &mut dyn FnMut(&str),
    ) -> Result<(), MapFnError> {
        // args.accept(CommandLineItem.expandToCommandLine(object))
        args(object.get_exec_path_string());
        Ok(())
    }
}

impl ExceptionlessMapFn<Artifact> for StructuredArgStripper {}

impl StrippingPathMapper {
    // Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:84 StrippingPathMapper.<init>
    fn new(primary_output: &Artifact, mnemonic: &str, is_starlark_action: bool) -> Self {
        // This is expected to always be "(bazel|blaze)-out".
        let output_root = primary_output.get_exec_path().sub_fragment(0, 1);
        let arg_stripper = StringStripper::new(output_root.get_path_string());
        // This kind of special handling should not be extended. It is a hack that works around a
        // limitation of the native implementation of location expansion: The output is just a
        // list of strings, not a structured command line that would allow transparent path
        // mapping. Instead, reimplement location expansion in Starlark and have it return an
        // Args object.
        let is_java_action = mnemonic == "Javac"
            || mnemonic == "JavacTurbine"
            || mnemonic == "Turbine"
            || mnemonic == "JavaResourceJar";
        let output_artifact_root = primary_output.get_root().clone();
        let stripped_output_artifact_root =
            MappedArtifactRoot::new(map_output_path(output_artifact_root.get_exec_path(), &output_root));
        StrippingPathMapper {
            output_root,
            mnemonic: mnemonic.to_string(),
            is_starlark_action,
            is_java_action,
            arg_stripper,
            output_artifact_root,
            stripped_output_artifact_root,
        }
    }

    /// Creates a new `PathMapper` that strips config prefixes if the particular action instance
    /// supports it; `None` if it does not.
    // Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:120 StrippingPathMapper.tryCreate
    pub fn try_create(
        action: &dyn Action,
        is_starlark_action: bool,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
    ) -> Option<StrippingPathMapper> {
        let primary_output = action.get_primary_output();
        let output_root = primary_output.get_exec_path().sub_fragment(0, 1);
        let mut inputs: Vec<Artifact> = action.get_inputs().to_list().to_vec();
        inputs.extend(
            action
                .get_additional_artifacts_for_path_mapping()
                .to_list()
                .iter()
                .cloned(),
        );
        if action.discovers_inputs() {
            inputs.extend(action.get_allowed_derived_inputs().to_list().iter().cloned());
        }
        inputs.extend(action.get_outputs());
        if is_path_strippable(&inputs, &output_root, input_metadata_provider) {
            return Some(StrippingPathMapper::new(
                &primary_output,
                &action.get_mnemonic(),
                is_starlark_action,
            ));
        }
        None
    }

    // Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:212 StrippingPathMapper.isSupported
    fn is_supported(&self, artifact: &Artifact) -> bool {
        // PORT: every action input reaching a Rust path mapper is an Artifact; a
        // `BasicActionInput` or `VirtualActionInput` (checked with isOutputPath in Java) does not.
        artifact.is_derived_artifact_instance()
    }
}

impl PathMapper for StrippingPathMapper {
    // Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:150 StrippingPathMapper.map
    fn map(&self, exec_path: &PathFragment) -> PathFragment {
        if is_output_path(exec_path, &self.output_root) {
            strip(exec_path)
        } else {
            exec_path.clone()
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:141 StrippingPathMapper.getMappedExecPathString
    fn get_mapped_exec_path_string(&self, artifact: &Artifact) -> String {
        if self.is_supported(artifact) {
            strip(artifact.get_exec_path()).get_path_string().to_string()
        } else {
            artifact.get_exec_path_string().to_string()
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:155 StrippingPathMapper.computeExecPathLengthDiff
    fn compute_exec_path_length_diff(&self, artifact: &Artifact) -> i32 {
        let unmapped_path = artifact.get_exec_path_string();
        // bazel-out/k8-fastbuild/... is mapped to bazel-out/${FIXED_CONFIG_SEGMENT}/...
        // Java `length()` and `indexOf`: chars, one per byte of an internal string.
        let first_slash = self.output_root.get_path_string().len() + 1;
        let second_slash = unmapped_path
            .get(first_slash + 1..)
            .and_then(|rest| rest.find('/'))
            .map(|i| (i + first_slash + 1) as i64)
            .unwrap_or(-1);
        (second_slash - first_slash as i64) as i32 - FIXED_CONFIG_SEGMENT.len() as i32
    }

    // Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:164 StrippingPathMapper.mapCustomStarlarkArgs
    fn map_custom_starlark_args<'a>(&self, chunk: Box<dyn ArgChunk + 'a>) -> Box<dyn ArgChunk + 'a> {
        if !self.is_starlark_action {
            return chunk;
        }
        // Add your favorite Starlark mnemonic that needs custom arg processing here.
        if !self.mnemonic.contains("Android")
            && self.mnemonic != "MergeManifests"
            && self.mnemonic != "StarlarkRClassGenerator"
            && self.mnemonic != "StarlarkAARGenerator"
            && self.mnemonic != "JetifySrcs"
            && self.mnemonic != "Desugar"
        {
            return chunk;
        }
        unported!(
            "StrippingPathMapper.mapCustomStarlarkArgs",
            "CustomStarlarkArgsIterator for the Android Starlark actions ({})",
            self.mnemonic
        )
    }

    // Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:186 StrippingPathMapper.getMapFn
    fn get_map_fn(&self, previous_flag: Option<&str>) -> &'static dyn ExceptionlessMapFn<Artifact> {
        if self.is_java_action
            && (previous_flag == Some("--javacopts") || previous_flag == Some("--resources"))
        {
            return &STRUCTURED_ARG_STRIPPER;
        }
        &blaze_collect::command_line_item::DEFAULT
    }

    // Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:197 StrippingPathMapper.mapHeuristically
    fn map_heuristically(&self, arg: &str) -> String {
        self.arg_stripper.strip(arg)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:202 StrippingPathMapper.mapRoot
    fn map_root(&self, artifact: &Artifact) -> FileRoot {
        if artifact.get_root() == &self.output_artifact_root {
            // The mapped root's path does not depend on the artifact, so we can share an
            // instance.
            return FileRoot::Mapped(self.stripped_output_artifact_root.clone());
        }
        // Fall back for source roots as well as runfiles tree artifacts, which should be very
        // rare.
        default_map_root(self, artifact)
    }
}

/// `PathMapper.super.mapRoot(artifact)` (the trait's default body).
// Java: com/google/devtools/build/lib/actions/PathMapper.java:159 PathMapper.mapRoot
fn default_map_root(mapper: &StrippingPathMapper, artifact: &Artifact) -> FileRoot {
    let root = artifact.get_root();
    if root.is_source_root() {
        return FileRoot::Mapped(MappedArtifactRoot::new(root.get_exec_path().clone()));
    }
    let exec_path = artifact.get_exec_path();
    let mapped_exec_path = mapper.map(exec_path);
    let root_relative_segment_count =
        exec_path.segment_count() - root.get_exec_path().segment_count();
    FileRoot::Mapped(MappedArtifactRoot::new(mapped_exec_path.sub_fragment(
        0,
        mapped_exec_path.segment_count() - root_relative_segment_count,
    )))
}

/// `map(execPath)` before the mapper exists (the constructor's `map(outputArtifactRoot.getExecPath())`).
fn map_output_path(exec_path: &PathFragment, output_root: &PathFragment) -> PathFragment {
    if is_output_path(exec_path, output_root) {
        strip(exec_path)
    } else {
        exec_path.clone()
    }
}

/// Utility class to strip output path configuration prefixes from arbitrary strings.
// Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:271 StrippingPathMapper.StringStripper
#[derive(Debug)]
struct StringStripper {
    output_root: String,
}

impl StringStripper {
    // Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:275 StrippingPathMapper.StringStripper.<init>
    fn new(output_root: &str) -> Self {
        StringStripper {
            output_root: output_root.to_string(),
        }
    }

    /// `pattern.matcher(str).replaceAll(outputRoot + "/" + FIXED_CONFIG_SEGMENT + "/")` with the
    /// pattern `outputRoot + "/[\\w_.-]+/"` (`stripPathsPattern`): an output root, a slash, a run
    /// of ASCII word characters, `.` and `-`, and a slash.
    ///
    /// Supports strings with multiple output paths in arbitrary places. Doesn't strip paths that
    /// would be non-existent without config prefixes (e.g. "bazel-out/x86-fastbuild",
    /// "bazel-out;foo", "/path/to/compiler bazel-out").
    // Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:292 StrippingPathMapper.StringStripper.stripPathsPattern
    // Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:300 StrippingPathMapper.StringStripper.strip
    // PORT: the output root is "bazel-out" or "blaze-out"; its regex metacharacters ('-') are
    // literal outside a character class, so the root is matched literally.
    fn strip(&self, s: &str) -> String {
        let prefix = format!("{}/", self.output_root);
        let replacement = format!("{}/{}/", self.output_root, FIXED_CONFIG_SEGMENT);
        let bytes = s.as_bytes();
        let mut out = String::with_capacity(s.len());
        let mut copied = 0;
        let mut i = 0;
        while i < s.len() {
            if s[i..].starts_with(&prefix) {
                let seg_start = i + prefix.len();
                let mut j = seg_start;
                // [\w_.-]+ : Java's \w is [a-zA-Z_0-9].
                while j < bytes.len()
                    && (bytes[j].is_ascii_alphanumeric()
                        || bytes[j] == b'_'
                        || bytes[j] == b'.'
                        || bytes[j] == b'-')
                {
                    j += 1;
                }
                // The greedy run cannot give back a '/', so the match needs one right after it.
                if j > seg_start && j < bytes.len() && bytes[j] == b'/' {
                    out.push_str(&s[copied..i]);
                    out.push_str(&replacement);
                    i = j + 1;
                    copied = i;
                    continue;
                }
            }
            // Advance one character (keep char boundaries).
            i += s[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        }
        out.push_str(&s[copied..]);
        out
    }
}

/// Is this a strippable path?
// Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:322 StrippingPathMapper.isOutputPath
fn is_output_path(path_fragment: &PathFragment, output_root: &PathFragment) -> bool {
    path_fragment.starts_with(output_root)
}

/// Is this action safe to strip? (Not two inputs in different configurations that would resolve
/// to the same path with different content.) Collisions between inputs from different
/// configurations that map to the same root-relative path are allowed if they have the same file
/// digest.
// Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:340 StrippingPathMapper.isPathStrippable
pub fn is_path_strippable(
    action_inputs: &[Artifact],
    output_root: &PathFragment,
    input_metadata_provider: Option<&dyn InputMetadataProvider>,
) -> bool {
    let mut root_relative_paths: HashMap<PathFragment, &Artifact> = HashMap::new();
    for input in action_inputs {
        if !is_output_path(input.get_exec_path(), output_root) {
            continue;
        }
        // For "bazel-out/k8-fastbuild/bin/foo/bar", get "bin/foo/bar".
        let key = input.get_exec_path().sub_fragment(2, input.get_exec_path().segment_count());
        let previous = *root_relative_paths.entry(key).or_insert(input);
        if previous != input && !have_same_digest(previous, input, input_metadata_provider) {
            return false;
        }
    }
    true
}

/// Returns true if the two inputs have the same file digest.
// Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:358 StrippingPathMapper.haveSameDigest
fn have_same_digest(
    _a: &Artifact,
    _b: &Artifact,
    input_metadata_provider: Option<&dyn InputMetadataProvider>,
) -> bool {
    if input_metadata_provider.is_none() {
        return false;
    }
    // PORT: the Rust `InputMetadataProvider` of this crate has no `getInputMetadata`; a
    // provider is only passed when a spawn is created, which does not create this mapper yet.
    unported!(
        "StrippingPathMapper.haveSameDigest",
        "comparing the digests of colliding inputs"
    )
}

/// Keep the config segment, but replace it with a fixed string to improve cacheability while
/// still preserving the general segment structure of the exec path.
// Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:380 StrippingPathMapper.strip
fn strip(exec_path: &PathFragment) -> PathFragment {
    exec_path
        .sub_fragment(0, 1)
        .get_relative(&PathFragment::create(FIXED_CONFIG_SEGMENT))
        .get_relative(&exec_path.sub_fragment(2, exec_path.segment_count()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The examples of the Java `stripPathsPattern` documentation.
    #[test]
    fn string_stripper_strips_output_paths() {
        let s = StringStripper::new("bazel-out");
        assert_eq!(
            s.strip("/path/to/compiler bazel-out/x86-fastbuild/foo src/my.src -Dbazel-out/arm-opt/bar"),
            "/path/to/compiler bazel-out/cfg/foo src/my.src -Dbazel-out/cfg/bar"
        );
        assert_eq!(s.strip("bazel-out/x86-fastbuild"), "bazel-out/x86-fastbuild");
        assert_eq!(s.strip("bazel-out;foo"), "bazel-out;foo");
        assert_eq!(s.strip("/path/to/compiler bazel-out"), "/path/to/compiler bazel-out");
    }

    #[test]
    fn strip_replaces_the_config_segment() {
        assert_eq!(
            strip(&PathFragment::create("bazel-out/k8-fastbuild/bin/a/b.jar")),
            PathFragment::create("bazel-out/cfg/bin/a/b.jar")
        );
    }
}
