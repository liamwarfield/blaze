//! Port of `lib/analysis/WorkspaceStatusAction.java` (analysis-time parts) and the
//! `WorkspaceStatusValue.BuildInfoKey` that owns the workspace status artifacts.

use std::any::Any;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::io::{self, Read};
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_collect::NestedSet;
use blaze_events::location::Location;
use blaze_vfs::Path;
use indexmap::IndexMap;

use crate::actions::action::{AbstractAction, Action};
use crate::actions::action_lookup::ActionLookupKey;
use crate::actions::action_owner::ActionOwner;
use crate::actions::artifact::Artifact;
use crate::actions::platform_info::{empty_platform_info, internal_platform};

/// `SkyFunctions.BUILD_INFO`, the function name of [`BuildInfoKey`].
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:128 SkyFunctions.BUILD_INFO
pub const BUILD_INFO_FUNCTION_NAME: &str = "BUILD_INFO";

/// `SkyKey` for `WorkspaceStatusValue`; the owner of the workspace status artifacts
/// (`WorkspaceStatusValue.BUILD_INFO_KEY` is its only instance).
///
/// PORT: Java's class lives in `lib/skyframe/WorkspaceStatusValue.java`; it is ported here, next
/// to the action that it owns, until the Skyframe side ports `WorkspaceStatusValue`. Code that
/// tests for it (`instanceof BuildInfoKey`) compares the function name with
/// [`BUILD_INFO_FUNCTION_NAME`], which only this key uses.
// Java: com/google/devtools/build/lib/skyframe/WorkspaceStatusValue.java:55 WorkspaceStatusValue.BuildInfoKey
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BuildInfoKey;

/// `WorkspaceStatusValue.BUILD_INFO_KEY`.
// Java: com/google/devtools/build/lib/skyframe/WorkspaceStatusValue.java:38 WorkspaceStatusValue.BUILD_INFO_KEY
pub const BUILD_INFO_KEY: BuildInfoKey = BuildInfoKey;

impl fmt::Display for BuildInfoKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // PORT: Java does not override toString (Object identity string); a stable name instead.
        f.write_str("BuildInfoKey")
    }
}

impl ActionLookupKey for BuildInfoKey {
    // Java: com/google/devtools/build/lib/skyframe/WorkspaceStatusValue.java:65 BuildInfoKey.getLabel
    fn get_label(&self) -> Option<Label> {
        None
    }

    // Java: com/google/devtools/build/lib/skyframe/WorkspaceStatusValue.java:71 BuildInfoKey.getConfigurationKey
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }

    // Java: com/google/devtools/build/lib/skyframe/WorkspaceStatusValue.java:59 BuildInfoKey.functionName
    fn function_name(&self) -> &'static str {
        BUILD_INFO_FUNCTION_NAME
    }

    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other.as_any().is::<BuildInfoKey>()
    }

    fn key_hash(&self) -> u64 {
        let mut h = DefaultHasher::new();
        BUILD_INFO_FUNCTION_NAME.hash(&mut h);
        h.finish()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `ActionOwner.SYSTEM_ACTION_OWNER`, the owner of system actions such as the workspace status
/// action.
// Java: com/google/devtools/build/lib/actions/ActionOwner.java:43 ActionOwner.SYSTEM_ACTION_OWNER
// PORT: defined here because the core `ActionOwner` port does not define the constant yet.
pub fn system_action_owner() -> ActionOwner {
    ActionOwner::create_dummy(
        /* label= */ Some(internal_platform()),
        Location::builtin().clone(),
        /* target_kind= */ "empty target kind",
        /* build_configuration_mnemonic= */ "system",
        /* configuration_checksum= */ "system",
        /* build_configuration_event= */ None,
        /* is_tool_configuration= */ false,
        /* execution_platform= */ Some(empty_platform_info()),
        /* aspect_descriptors= */ Vec::new(),
        /* exec_properties= */ IndexMap::new(),
    )
}

// PORT: `WorkspaceStatusAction.Options` (`--embed_label`, `--workspace_status_command`) and its
// `OneLineStringConverter` are transcribed in blaze_optdefs
// (`command::workspace_status_action::WorkspaceStatusActionOptions`); the actions take the option
// values as plain parameters. `WorkspaceStatusAction.Context` (the action context) belongs to
// execution (wave W6).

/// Parses the output of the workspace status action.
///
/// The output is a text file with each line representing a workspace status info key. The key is
/// the part of the line before the first space and should consist of the characters `[A-Z_]`
/// (although this is not checked). Everything after the first space is the value.
// Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:120 WorkspaceStatusAction.parseValues
// PORT: java-hash-order. Java returns `ImmutableMap.copyOf(HashMap)`, whose iteration order is
// the Java HashMap order; its only consumer (`BuildInfoFileWriteAction.execute`, wave W6) copies it
// into another HashMap before handing it to Starlark, and must replicate that order there.
pub fn parse_values(file: &Path) -> io::Result<HashMap<String, String>> {
    let mut bytes = Vec::new();
    file.get_input_stream()?.read_to_end(&mut bytes)?;
    Ok(parse_values_content(&bytes))
}

/// The parsing part of [`parse_values`], on the file's bytes.
// Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:120 WorkspaceStatusAction.parseValues
pub fn parse_values_content(bytes: &[u8]) -> HashMap<String, String> {
    let mut result = HashMap::new();
    // Java: `new String(FileSystemUtils.readContentAsLatin1(file))`. Until the decoding switch
    // (`string_encoding::STRINGS_ARE_INTERNAL`) a status file that is not valid UTF-8 has no
    // string.
    let content = match blaze_util::string_encoding::internal_string_from_bytes(bytes) {
        Ok(s) => s,
        Err(_) => blaze_util::unported!(
            "WorkspaceStatusAction.parseValues",
            "status file is not valid UTF-8"
        ),
    };
    // Splitter.on('\n') and Splitter.on(' ').limit(2).
    for line in content.split('\n') {
        let items: Vec<&str> = line.splitn(2, ' ').collect();
        if items.len() != 2 {
            continue;
        }
        result.insert(items[0].to_string(), items[1].to_string());
    }
    result
}

/// Environment for the [`WorkspaceStatusActionFactory`] to create the workspace status action.
// Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:137 WorkspaceStatusAction.Environment
pub trait WorkspaceStatusActionEnvironment {
    // Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:138 Environment.createStableArtifact
    fn create_stable_artifact(&self, name: &str) -> Artifact;

    // Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:140 Environment.createVolatileArtifact
    fn create_volatile_artifact(&self, name: &str) -> Artifact;
}

/// Factory for [`WorkspaceStatusAction`].
// Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:144 WorkspaceStatusAction.Factory
pub trait WorkspaceStatusActionFactory: Send + Sync {
    /// Creates the workspace status action.
    ///
    /// The action is never re-created, but the same action object is executed on every build.
    // Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:151 Factory.createWorkspaceStatusAction
    fn create_workspace_status_action(
        &self,
        env: &dyn WorkspaceStatusActionEnvironment,
    ) -> Arc<dyn WorkspaceStatusAction>;

    /// Returns a map containing any available workspace status information.
    ///
    /// Used to construct a `BuildInfoEvent` at the end of builds in which no such event was
    /// posted.
    // Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:159 Factory.createDummyWorkspaceStatus
    // PORT: the `@Nullable WorkspaceInfoFromDiff` parameter is not ported (no Bazel
    // implementation reads it).
    fn create_dummy_workspace_status(&self) -> BTreeMap<String, String>;
}

/// An action writing the workspace status files.
///
/// These files represent information about the environment the build was run in. They are used
/// by language-specific build info factories to make the data in them available for individual
/// languages (e.g. by turning them into .h files for C++)
///
/// The format of these files a list of key-value pairs, one for each line. The key and the value
/// are separated by a space.
///
/// There are two of these files: volatile and stable. Changes in the volatile file do not cause
/// rebuilds if no other file is changed. This is useful for frequently-changing information that
/// does not significantly affect the build, e.g. the current time.
///
/// Implementors embed the `AbstractAction` from [`new_workspace_status_action`] and, in their
/// [`Action`] impl, override `execute_unconditionally` and `is_volatile` to return `true` and
/// leave `compute_key` empty (Java's are final).
// Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:67 WorkspaceStatusAction
pub trait WorkspaceStatusAction: Action {
    /// The volatile status artifact containing items that may change even if nothing changed
    /// between the two builds, e.g. current time.
    // Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:178 WorkspaceStatusAction.getVolatileStatus
    fn get_volatile_status(&self) -> Artifact;

    /// The stable status artifact containing items that change only if information relevant to
    /// the build changes, e.g. the name of the user running the build or the hostname.
    // Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:184 WorkspaceStatusAction.getStableStatus
    fn get_stable_status(&self) -> Artifact;

    /// The `workspaceStatusDescription` given to the constructor (used in error messages).
    fn get_workspace_status_description(&self) -> &str;
}

/// `WorkspaceStatusAction(owner, inputs, outputs, workspaceStatusDescription)`: the
/// `AbstractAction` part (the description is kept by the implementor).
// Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:165 WorkspaceStatusAction.<init>
pub fn new_workspace_status_action(
    owner: ActionOwner,
    inputs: NestedSet<Artifact>,
    outputs: Vec<Artifact>,
) -> AbstractAction {
    AbstractAction::new(owner, inputs, outputs)
}

/// The message of `createExecutionException`: `"Failed to determine " + description + ": " +
/// e.getMessage()`.
// Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:205 WorkspaceStatusAction.createExecutionException
pub fn execution_exception_message(workspace_status_description: &str, message: &str) -> String {
    format!(
        "Failed to determine {}: {}",
        workspace_status_description, message
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_values_splits_on_first_space() {
        let m = parse_values_content(b"BUILD_USER alice smith\nNOVALUE\nEMPTY \n\n");
        assert_eq!(m.get("BUILD_USER").map(String::as_str), Some("alice smith"));
        assert_eq!(m.get("EMPTY").map(String::as_str), Some(""));
        assert!(!m.contains_key("NOVALUE"));
        assert_eq!(m.len(), 2);
    }
}
