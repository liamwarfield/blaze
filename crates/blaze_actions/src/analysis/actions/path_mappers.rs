//! Port of `lib/analysis/actions/PathMappers.java`: utility methods that are the canonical way
//! for actions to support path mapping (see `PathMapper`).

use std::sync::Arc;

use blaze_collect::NestedSet;
use blaze_util::Fingerprint;
use indexmap::IndexMap;

use crate::actions::action::{Action, ComputeKeyError, InputMetadataProvider};
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::artifact::Artifact;
use crate::actions::path_mapper::{NoopPathMapper, OutputPathsMode, PathMapper};

// TODO: Remove actions from this list by adding ExecutionRequirements.SUPPORTS_PATH_MAPPING to
//  their execution info instead.
// Java: com/google/devtools/build/lib/analysis/actions/PathMappers.java:46 PathMappers.SUPPORTED_MNEMONICS
const SUPPORTED_MNEMONICS: &[&str] = &[
    "AndroidLint",
    "CompileAndroidResources",
    "DeJetify",
    "DejetifySrcs",
    "Desugar",
    "DexBuilder",
    "Jetify",
    "JetifySrcs",
    "LinkAndroidResources",
    "MergeAndroidAssets",
    "MergeManifests",
    "ParseAndroidResources",
    "StarlarkAARGenerator",
    "StarlarkMergeCompiledAndroidResources",
    "StarlarkRClassGenerator",
    "Mock action",
];

// PORT: the ExecutionRequirements constants used here (execution_requirements.rs is owned by
// another port).
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:305 ExecutionRequirements.LOCAL
const LOCAL: &str = "local";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:333 ExecutionRequirements.NO_REMOTE
const NO_REMOTE: &str = "no-remote";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:342 ExecutionRequirements.NO_SANDBOX
const NO_SANDBOX: &str = "no-sandbox";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:403 ExecutionRequirements.SUPPORTS_PATH_MAPPING
pub(crate) const SUPPORTS_PATH_MAPPING: &str = "supports-path-mapping";

/// `StrippingPathMapper.GUID`.
// Java: com/google/devtools/build/lib/analysis/actions/StrippingPathMapper.java:88 StrippingPathMapper.GUID
pub const STRIPPING_PATH_MAPPER_GUID: &str = "8eb2ad5a-85d4-435b-858f-5c192e91997d";

/// Actions that support path mapping should call this method from `getKey`.
///
/// Compared to [`create`], this method does not flatten nested sets and thus can't result in
/// memory regressions.
// Java: com/google/devtools/build/lib/analysis/actions/PathMappers.java:76 PathMappers.addToFingerprint
pub fn add_to_fingerprint(
    mnemonic: &str,
    execution_info: &IndexMap<String, String>,
    additional_artifacts_for_path_mapping: &NestedSet<Artifact>,
    action_key_context: &ActionKeyContext,
    output_paths_mode: OutputPathsMode,
    fingerprint: &mut Fingerprint,
) -> Result<(), ComputeKeyError> {
    // Creating a new PathMapper instance can be expensive, but isn't needed here: Whether and
    // how path mapping applies to the action only depends on the output paths mode and the action
    // inputs, which are already part of the action key.
    let effective_output_paths_mode =
        get_effective_output_paths_mode(output_paths_mode, mnemonic, execution_info);
    if effective_output_paths_mode == OutputPathsMode::STRIP {
        fingerprint.add_string(STRIPPING_PATH_MAPPER_GUID);
        // These artifacts are not part of the actual command line or inputs, but influence the
        // behavior of path mapping.
        action_key_context
            .add_nested_set_to_fingerprint(fingerprint, additional_artifacts_for_path_mapping)?;
    }
    Ok(())
}

/// Actions that support path mapping should call this method when creating their `Spawn`.
///
/// The returned `PathMapper` has to be passed to `CommandLine::arguments_with`,
/// `CommandLines::expand` or any other variants of these functions.
///
/// Note: This method flattens nested sets and should thus not be called from methods that are
/// executed in the analysis phase.
///
/// Returns a `PathMapper` that maps paths of the action's inputs and outputs. May be
/// `PathMapper.NOOP` if path mapping is not applicable to the action.
// Java: com/google/devtools/build/lib/analysis/actions/PathMappers.java:124 PathMappers.create
pub fn create(
    action: &dyn Action,
    output_paths_mode: OutputPathsMode,
    is_starlark_action: bool,
    input_metadata_provider: Option<&dyn InputMetadataProvider>,
) -> Arc<dyn PathMapper> {
    if get_effective_output_paths_mode(
        output_paths_mode,
        &action.get_mnemonic(),
        &action.get_execution_info(),
    ) != OutputPathsMode::STRIP
    {
        return Arc::new(NoopPathMapper);
    }
    match super::stripping_path_mapper::StrippingPathMapper::try_create(
        action,
        is_starlark_action,
        input_metadata_provider,
    ) {
        Some(mapper) => Arc::new(mapper),
        None => Arc::new(NoopPathMapper),
    }
}

/// `PathMappers.create` for an action that is about to run: the same mapper, but a stripping
/// mapper stops with `unported!`.
///
/// PORT: running a path-mapped spawn needs `Spawn.getPathMapper()` (the executor's staging of
/// the mapped inputs and outputs, the `.jdeps` rewriting of `JavaCompileAction`), which is not
/// ported: the spawns are created with `SpawnPathMapper::NOOP`. Argument display (`aquery`,
/// `getArguments`) uses [`create`].
// Java: com/google/devtools/build/lib/analysis/actions/PathMappers.java:124 PathMappers.create
pub fn create_for_execution(
    action: &dyn Action,
    output_paths_mode: OutputPathsMode,
    is_starlark_action: bool,
    input_metadata_provider: Option<&dyn InputMetadataProvider>,
) -> Arc<dyn PathMapper> {
    let mapper = create(
        action,
        output_paths_mode,
        is_starlark_action,
        input_metadata_provider,
    );
    if !mapper.is_noop() {
        blaze_util::unported!(
            "Spawn.getPathMapper",
            "executing a path-mapped spawn (--experimental_output_paths=strip, {})",
            action.get_mnemonic()
        );
    }
    mapper
}

// PORT: `getOutputPathsMode(BuildConfigurationValue)` reads `CoreOptions.outputPathsMode` from a
// configuration, which this crate cannot see; callers read the option themselves (null
// configuration → OFF).

/// Returns the effective `OutputPathsMode` for an action based on the action's mnemonic and
/// execution info. This may return a mode other than `OFF` even though path mapping will be
/// disabled during execution due to path collisions.
// Java: com/google/devtools/build/lib/analysis/actions/PathMappers.java:158 PathMappers.getEffectiveOutputPathsMode
pub fn get_effective_output_paths_mode(
    output_paths_mode: OutputPathsMode,
    mnemonic: &str,
    execution_info: &IndexMap<String, String>,
) -> OutputPathsMode {
    if execution_info.contains_key(LOCAL)
        || (execution_info.contains_key(NO_SANDBOX) && execution_info.contains_key(NO_REMOTE))
    {
        // Path mapping requires sandboxed or remote execution.
        return OutputPathsMode::OFF;
    }
    if output_paths_mode == OutputPathsMode::STRIP
        && (SUPPORTED_MNEMONICS.contains(&mnemonic)
            || execution_info.contains_key(SUPPORTS_PATH_MAPPING))
    {
        return OutputPathsMode::STRIP;
    }
    OutputPathsMode::OFF
}
