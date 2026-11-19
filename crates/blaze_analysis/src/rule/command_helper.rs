//! Ports of `analysis/CommandHelper.java`, `analysis/BashCommandConstructor.java` and
//! `FileWriteAction.createFile`.

use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_actions::actions::execution_requirements::REQUIRES_DARWIN;
use blaze_actions::analysis::actions::file_write_action::FileWriteAction;
use blaze_actions::{ActionRef, Artifact};
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder};
use blaze_shell::util::os::OS;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use crate::configured_target::ConfiguredTarget;
use crate::providers::alias_provider::AliasProvider;
use crate::providers::file_provider::FileProvider;
use crate::rule::label_expander;
use crate::rule::location_expander::LocationExpander;
use crate::rule::rule_context::RuleContext;
use crate::rule::rule_error_consumer::RuleErrorConsumer;

/// The class for constructing command line for Bash.
///
/// PORT: Java's `CommandConstructor` interface has two more implementations
/// (`WindowsBatchCommandConstructor`, `WindowsPowershellCommandConstructor`); skipped, Windows
/// only. Callers that would need them fail with `unported!`.
// Java: com/google/devtools/build/lib/analysis/BashCommandConstructor.java:23 BashCommandConstructor
#[derive(Clone, Debug)]
pub struct BashCommandConstructor {
    shell_path: PathFragment,
    script_name_suffix: String,
}

impl BashCommandConstructor {
    // Java: com/google/devtools/build/lib/analysis/BashCommandConstructor.java:28 BashCommandConstructor.<init>
    fn new(shell_path: PathFragment, script_name_suffix: String) -> BashCommandConstructor {
        BashCommandConstructor {
            shell_path,
            script_name_suffix,
        }
    }

    /// The shell this constructor runs commands with.
    pub fn get_shell_path(&self) -> &PathFragment {
        &self.shell_path
    }

    // Java: com/google/devtools/build/lib/analysis/BashCommandConstructor.java:34 BashCommandConstructor.asExecArgv
    pub fn as_exec_argv_for_script(&self, script_file_artifact: &Artifact) -> Vec<String> {
        vec![
            self.shell_path.get_path_string().to_owned(),
            script_file_artifact.get_exec_path_string().to_owned(),
        ]
    }

    // Java: com/google/devtools/build/lib/analysis/BashCommandConstructor.java:39 BashCommandConstructor.asExecArgv
    pub fn as_exec_argv(&self, command: &str) -> Vec<String> {
        vec![
            self.shell_path.get_path_string().to_owned(),
            "-c".to_owned(),
            command.to_owned(),
        ]
    }

    /// Writes `command` to an executable script next to the target and returns the script.
    // Java: com/google/devtools/build/lib/analysis/BashCommandConstructor.java:44 BashCommandConstructor.commandAsScript
    pub fn command_as_script(&self, rule_context: &RuleContext, command: &str) -> Artifact {
        let (script_file_artifact, action) =
            self.command_as_script_unregistered(rule_context, command);
        rule_context.register_action(action);
        script_file_artifact
    }

    /// `commandAsScript` without the registration of the script's `FileWriteAction`: returns the
    /// script artifact and the action writing it, for callers that register actions in order
    /// later (the Starlark action factory's pending list).
    ///
    /// PORT: Java registers the action inside `FileWriteAction.createFile`; a caller of this
    /// variant must register the returned action at the point where Java calls
    /// `commandAsScript`.
    // Java: com/google/devtools/build/lib/analysis/BashCommandConstructor.java:44 BashCommandConstructor.commandAsScript
    pub fn command_as_script_unregistered(
        &self,
        rule_context: &RuleContext,
        command: &str,
    ) -> (Artifact, ActionRef) {
        let script_file_name = format!(
            "{}{}",
            rule_context.get_target().get_name(),
            self.script_name_suffix
        );
        let script_file_contents = format!("#!/bin/bash\n{command}");
        create_file_unregistered(
            rule_context,
            &script_file_name,
            script_file_contents,
            /* executable= */ true,
        )
    }
}

/// Creates an artifact in the genfiles directory of the rule's package and registers a
/// `FileWriteAction` writing `contents` to it.
// Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:84 FileWriteAction.createFile
pub fn create_file(
    rule_context: &RuleContext,
    file_name: &str,
    contents: String,
    executable: bool,
) -> Artifact {
    let (script_file_artifact, action) =
        create_file_unregistered(rule_context, file_name, contents, executable);
    rule_context.register_action(action);
    script_file_artifact
}

/// `FileWriteAction.createFile` without `ruleContext.registerAction`: the artifact and the
/// action writing it (see [`BashCommandConstructor::command_as_script_unregistered`]).
// Java: com/google/devtools/build/lib/analysis/actions/FileWriteAction.java:84 FileWriteAction.createFile
pub fn create_file_unregistered(
    rule_context: &RuleContext,
    file_name: &str,
    contents: String,
    executable: bool,
) -> (Artifact, ActionRef) {
    let script_file_artifact = rule_context.get_package_relative_artifact(
        &PathFragment::create(file_name),
        &rule_context.get_genfiles_directory(),
    );
    let action: ActionRef = Arc::new(FileWriteAction::create_from_context(
        rule_context.get_action_owner(),
        script_file_artifact.clone(),
        contents,
        executable,
    ));
    (script_file_artifact, action)
}

/// Maximum total command-line length, in bytes, not counting "/bin/bash -c ". If the command is
/// very long, then we write the command to a script file, to avoid overflowing any limits on
/// command-line length. For short commands, we just use /bin/bash -c command.
///
/// Maximum command line length on Windows is 32767\[1\], but for cmd.exe it is 8192\[2\]. \[1\]
/// https://msdn.microsoft.com/en-us/library/ms682425(VS.85).aspx \[2\]
/// https://support.microsoft.com/en-us/kb/830473.
// Java: com/google/devtools/build/lib/analysis/CommandHelper.java:217 CommandHelper.maxCommandLength
// PORT: skipped, `setMaxCommandLengthForTesting` (a test-only override no production code sets).
pub fn max_command_length(execution_os: OS) -> usize {
    if execution_os == OS::Windows {
        8000
    } else {
        64000
    }
}

/// Java `command.length()` of the command string, an internal string: its chars (one per byte
/// of the original text; analysis review R2-5).
// Java: com/google/devtools/build/lib/analysis/CommandHelper.java:262 CommandHelper.buildCommandLine
pub(crate) fn java_string_length(s: &str) -> usize {
    s.chars().count()
}

/// If `command` is too long, creates a helper shell script that runs that command.
///
/// Returns the `Artifact` corresponding to that script.
///
/// Otherwise, when `command` is shorter than the platform's shell's command length limit, this
/// method does nothing and returns `None`.
// Java: com/google/devtools/build/lib/analysis/CommandHelper.java:281 CommandHelper.commandHelperScriptMaybe
pub fn command_helper_script_maybe(
    rule_context: &RuleContext,
    command: &str,
    constructor: &BashCommandConstructor,
    execution_os: OS,
) -> Option<Artifact> {
    if java_string_length(command) <= max_command_length(execution_os) {
        None
    } else {
        Some(constructor.command_as_script(rule_context, command))
    }
}

/// `commandHelperScriptMaybe` that does not register the helper script's `FileWriteAction`:
/// returns the script artifact together with the action, which the caller registers at the point
/// where Java calls `commandHelperScriptMaybe` (`StarlarkActionFactory.runShell` registers it
/// through its ordered pending list, so the script action keeps its place between the actions
/// created before and after it).
// Java: com/google/devtools/build/lib/analysis/CommandHelper.java:281 CommandHelper.commandHelperScriptMaybe
pub fn command_helper_script_maybe_unregistered(
    rule_context: &RuleContext,
    command: &str,
    constructor: &BashCommandConstructor,
    execution_os: OS,
) -> Option<(Artifact, ActionRef)> {
    if java_string_length(command) <= max_command_length(execution_os) {
        None
    } else {
        Some(constructor.command_as_script_unregistered(rule_context, command))
    }
}

/// The `Map<String, String> executionInfo` argument of `shellPath`: only `containsKey` is used.
pub trait ExecutionInfoKeys {
    fn contains_execution_info_key(&self, key: &str) -> bool;
}

impl ExecutionInfoKeys for BTreeMap<String, String> {
    fn contains_execution_info_key(&self, key: &str) -> bool {
        self.contains_key(key)
    }
}

impl ExecutionInfoKeys for IndexMap<String, String> {
    fn contains_execution_info_key(&self, key: &str) -> bool {
        self.contains_key(key)
    }
}

/// Returns the path to the shell for an action with the given execution requirements.
// Java: com/google/devtools/build/lib/analysis/CommandHelper.java:324 CommandHelper.shellPath
fn shell_path<M: ExecutionInfoKeys + ?Sized>(
    execution_info: &M,
    sh_executable: &PathFragment,
) -> PathFragment {
    // Use vanilla /bin/bash for actions running on mac machines.
    if execution_info.contains_execution_info_key(REQUIRES_DARWIN) {
        PathFragment::create("/bin/bash")
    } else {
        sh_executable.clone()
    }
}

// Java: com/google/devtools/build/lib/analysis/CommandHelper.java:332 CommandHelper.buildBashCommandConstructor
pub fn build_bash_command_constructor<M: ExecutionInfoKeys + ?Sized>(
    execution_info: &M,
    sh_executable: &PathFragment,
    script_post_fix: &str,
) -> BashCommandConstructor {
    BashCommandConstructor::new(
        shell_path(execution_info, sh_executable),
        script_post_fix.to_owned(),
    )
}

/// Builder class to assist with creating an instance of `CommandHelper`. The Builder can
/// optionally add additional tools as dependencies, and a map of labels to be resolved.
// Java: com/google/devtools/build/lib/analysis/CommandHelper.java:62 CommandHelper.Builder
pub struct CommandHelperBuilder<'a> {
    rule_context: &'a RuleContext,
    tool_dependencies: Vec<Vec<ConfiguredTarget>>,
    /// Java `ImmutableMap.Builder<Label, Iterable<Artifact>>`: entries in insertion order,
    /// duplicate keys rejected by `buildOrThrow`.
    label_map: Vec<(Label, Vec<Artifact>)>,
}

impl<'a> CommandHelperBuilder<'a> {
    // Java: com/google/devtools/build/lib/analysis/CommandHelper.java:68 CommandHelper.Builder.<init>
    fn new(rule_context: &'a RuleContext) -> CommandHelperBuilder<'a> {
        CommandHelperBuilder {
            rule_context,
            tool_dependencies: Vec::new(),
            label_map: Vec::new(),
        }
    }

    /// Adds tools, as a set of executable binaries, by fetching them from the given attribute on
    /// the `rule_context`. Populates manifests, remoteRunfiles and label map where required.
    // Java: com/google/devtools/build/lib/analysis/CommandHelper.java:76 CommandHelper.Builder.addToolDependencies
    pub fn add_tool_dependencies_attribute(mut self, tool_attribute_name: &str) -> Self {
        let dependencies = self.rule_context.get_prerequisites(tool_attribute_name);
        self.tool_dependencies.push(dependencies);
        self
    }

    /// Adds tools, as a set of executable binaries. Populates manifests, remoteRunfiles and label
    /// map where required.
    // Java: com/google/devtools/build/lib/analysis/CommandHelper.java:87 CommandHelper.Builder.addToolDependencies
    pub fn add_tool_dependencies(mut self, tool_dependencies: Vec<ConfiguredTarget>) -> Self {
        self.tool_dependencies.push(tool_dependencies);
        self
    }

    /// Adds files to set of known files of label. Used for resolving $(location) variables.
    // Java: com/google/devtools/build/lib/analysis/CommandHelper.java:95 CommandHelper.Builder.addLabelMap
    pub fn add_label_map(mut self, label_map: IndexMap<Label, Vec<Artifact>>) -> Self {
        self.label_map.extend(label_map);
        self
    }

    /// Returns the built `CommandHelper`.
    // Java: com/google/devtools/build/lib/analysis/CommandHelper.java:101 CommandHelper.Builder.build
    pub fn build(self) -> CommandHelper<'a> {
        // Java: ImmutableMap.Builder.buildOrThrow.
        let mut label_map: IndexMap<Label, Vec<Artifact>> = IndexMap::new();
        for (key, value) in self.label_map {
            if label_map.contains_key(&key) {
                panic!("IllegalArgumentException: Multiple entries with same key: {key}");
            }
            label_map.insert(key, value);
        }
        CommandHelper::new(self.rule_context, self.tool_dependencies, label_map)
    }
}

/// Provides shared functionality for parameterized command-line launching. Also used by
/// `com.google.devtools.build.lib.rules.extra.ExtraActionFactory`.
///
/// Two largely independent separate sets of functionality are provided: 1- string interpolation
/// for `$(location[s] ...)` and `$(MakeVariable)` 2- a utility to build potentially large command
/// lines (presumably made of multiple commands), that if presumed too large for the kernel's
/// taste can be dumped into a shell script that will contain the same commands, at which point
/// the shell script is added to the list of inputs.
// Java: com/google/devtools/build/lib/analysis/CommandHelper.java:48 CommandHelper
pub struct CommandHelper<'a> {
    /// Use labelMap for heuristically expanding labels (does not include "outs") This is similar
    /// to heuristic location expansion in LocationExpander and should be kept in sync.
    label_map: IndexMap<Label, Vec<Artifact>>,

    /// The ruleContext this helper works on
    rule_context: &'a RuleContext,

    /// Output executable files from the 'tools' attribute.
    resolved_tools: NestedSet<Artifact>,
}

impl<'a> CommandHelper<'a> {
    /// Returns a new `CommandHelperBuilder` to create a `CommandHelper` based on the given
    /// `RuleContext`.
    // Java: com/google/devtools/build/lib/analysis/CommandHelper.java:54 CommandHelper.builder
    pub fn builder(rule_context: &'a RuleContext) -> CommandHelperBuilder<'a> {
        CommandHelperBuilder::new(rule_context)
    }

    /// Creates a `CommandHelper`.
    ///
    /// `tools_list` resolves sets of tools into set of executable binaries. Populates manifests,
    /// remoteRunfiles and label map where required. `label_map` adds files to set of known files
    /// of label. Used for resolving $(location) variables.
    // Java: com/google/devtools/build/lib/analysis/CommandHelper.java:127 CommandHelper.<init>
    fn new(
        rule_context: &'a RuleContext,
        tools_list: Vec<Vec<ConfiguredTarget>>,
        label_map: IndexMap<Label, Vec<Artifact>>,
    ) -> CommandHelper<'a> {
        let mut resolved_tools_builder: NestedSetBuilder<Artifact> =
            NestedSetBuilder::stable_order();
        // PORT: Java `HashMap<Label, Collection<Artifact>>` with `HashSet` values, copied into an
        // `ImmutableMap` of `ImmutableList`s in hash iteration order. That order is never
        // observable: the map is only searched by label, `$(location)` sorts the paths
        // (LocationExpander.java:299 getPaths) and heuristic label expansion requires exactly one
        // artifact (LabelExpander.java:127). Insertion order is kept here.
        let mut temp_label_map: IndexMap<Label, Vec<Artifact>> = IndexMap::new();

        for (key, value) in &label_map {
            let collection = Self::map_get(&mut temp_label_map, key.clone());
            for artifact in value {
                Self::set_add(collection, artifact.clone());
            }
        }

        for tools in &tools_list {
            for dep in tools {
                // (Note: exec configuration)
                let Some(tool) = crate::configuredtargets::get_files_to_run_provider(dep) else {
                    continue;
                };

                let files_to_build: NestedSet<Artifact> =
                    crate::configuredtargets::get_provider::<FileProvider>(dep)
                        .expect("NullPointerException: FileProvider")
                        .get_files_to_build()
                        .clone();
                resolved_tools_builder.add_transitive(&files_to_build);

                let executable_artifact = tool.get_executable();
                let label = AliasProvider::get_dependency_label(dep);

                // If the label has an executable artifact add that to the multimaps.
                if let Some(executable_artifact) = executable_artifact {
                    Self::set_add(
                        Self::map_get(&mut temp_label_map, label),
                        executable_artifact.clone(),
                    );
                    // Also send the runfiles if needed.
                    if let Some(runfiles_support) = tool.get_runfiles_support() {
                        resolved_tools_builder
                            .add(runfiles_support.get_runfiles_tree_artifact().clone());
                        // It's possible that getExecutable() returns an artifact that is not in
                        // getFilesToBuild(). It is not nice, but it happens
                        // (see test_executable_without_default_files)
                        resolved_tools_builder.add(runfiles_support.get_executable().clone());
                    }
                } else {
                    // Map all depArtifacts to the respective label using the multimaps.
                    let collection = Self::map_get(&mut temp_label_map, label);
                    for artifact in files_to_build.to_list().iter() {
                        Self::set_add(collection, artifact.clone());
                    }
                }
            }
        }

        CommandHelper {
            label_map: temp_label_map,
            rule_context,
            resolved_tools: resolved_tools_builder.build(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/CommandHelper.java:184 CommandHelper.getResolvedTools
    pub fn get_resolved_tools(&self) -> &NestedSet<Artifact> {
        &self.resolved_tools
    }

    // Java: com/google/devtools/build/lib/analysis/CommandHelper.java:188 CommandHelper.getLabelMap
    pub fn get_label_map(&self) -> &IndexMap<Label, Vec<Artifact>> {
        &self.label_map
    }

    // Returns the value in the specified corresponding to 'key', creating and
    // inserting an empty container if absent.  We use Map not Multimap because
    // we need to distinguish the cases of "empty value" and "absent key".
    // Java: com/google/devtools/build/lib/analysis/CommandHelper.java:195 CommandHelper.mapGet
    fn map_get(map: &mut IndexMap<Label, Vec<Artifact>>, key: Label) -> &mut Vec<Artifact> {
        // We use sets not lists, because it's conceivable that the same artifact
        // could appear twice, e.g. in "srcs" and "deps".
        map.entry(key).or_default()
    }

    /// `HashSet.add` on a collection returned by `map_get`.
    fn set_add(collection: &mut Vec<Artifact>, artifact: Artifact) {
        if !collection.contains(&artifact) {
            collection.push(artifact);
        }
    }

    /// Resolves a command, and expands known locations for $(location) variables.
    // Only exists to support a legacy Starlark API.
    // Java: com/google/devtools/build/lib/analysis/CommandHelper.java:226 CommandHelper.resolveCommandAndExpandLabels
    pub fn resolve_command_and_expand_labels(
        &self,
        command: &str,
        attribute: Option<&str>,
        allow_data_in_label: bool,
    ) -> String {
        let expander = if allow_data_in_label {
            LocationExpander::with_exec_paths_and_data(self.rule_context, self.label_map.clone())
        } else {
            LocationExpander::with_exec_paths(self.rule_context, self.label_map.clone())
        };
        match attribute {
            Some(attribute) => expander.expand_attribute(attribute, command),
            None => expander.expand(command),
        }
    }

    /// Expands labels occurring in the string "expr" in the rule 'cmd'. Each label must be valid,
    /// be a declared prerequisite, and expand to a unique path.
    ///
    /// If the expansion fails, an attribute error is reported and the original expression is
    /// returned.
    // Java: com/google/devtools/build/lib/analysis/CommandHelper.java:249 CommandHelper.expandLabelsHeuristically
    pub fn expand_labels_heuristically(&self, expr: &str) -> String {
        match label_expander::expand(Some(expr), &self.label_map, self.rule_context.get_label()) {
            Ok(expansion) => expansion,
            Err(nuee) => {
                self.rule_context.attribute_error("cmd", nuee.get_message());
                expr.to_owned()
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/CommandHelper.java:258 CommandHelper.buildCommandLineMaybeWithScriptFile
    fn build_command_line_maybe_with_script_file(
        rule_context: &RuleContext,
        command: &str,
        constructor: &BashCommandConstructor,
        execution_os: OS,
    ) -> (Vec<String>, Option<Artifact>) {
        if java_string_length(command) <= max_command_length(execution_os) {
            (constructor.as_exec_argv(command), None)
        } else {
            // Use script file.
            let script_file_artifact = constructor.command_as_script(rule_context, command);
            let argv = constructor.as_exec_argv_for_script(&script_file_artifact);
            (argv, Some(script_file_artifact))
        }
    }

    /// Builds the set of command-line arguments using the specified shell path. Creates a bash
    /// script if the command line is longer than the allowed maximum `max_command_length`. Fixes
    /// up the input artifact list with the created bash script when required.
    // Java: com/google/devtools/build/lib/analysis/CommandHelper.java:296 CommandHelper.buildCommandLine
    pub fn build_command_line(
        &self,
        command: &str,
        inputs: &mut NestedSetBuilder<Artifact>,
        constructor: &BashCommandConstructor,
        execution_os: OS,
    ) -> Vec<String> {
        let (argv, script_file) = Self::build_command_line_maybe_with_script_file(
            self.rule_context,
            command,
            constructor,
            execution_os,
        );
        if let Some(script_file) = script_file {
            inputs.add(script_file);
        }
        argv
    }

    /// Builds the set of command-line arguments. Creates a bash script if the command line is
    /// longer than the allowed maximum `max_command_length`. Fixes up the input artifact list
    /// with the created bash script when required.
    // Java: com/google/devtools/build/lib/analysis/CommandHelper.java:314 CommandHelper.buildCommandLine
    pub fn build_command_line_list(
        &self,
        command: &str,
        inputs: &mut Vec<Artifact>,
        constructor: &BashCommandConstructor,
        execution_os: OS,
    ) -> Vec<String> {
        let (argv, script_file) = Self::build_command_line_maybe_with_script_file(
            self.rule_context,
            command,
            constructor,
            execution_os,
        );
        if let Some(script_file) = script_file {
            inputs.push(script_file);
        }
        argv
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_path_uses_bin_bash_for_darwin() {
        let mut info: BTreeMap<String, String> = BTreeMap::new();
        let sh = PathFragment::create("/usr/bin/sh");
        assert_eq!(
            build_bash_command_constructor(&info, &sh, ".sh").get_shell_path(),
            &sh
        );
        info.insert(REQUIRES_DARWIN.to_owned(), String::new());
        assert_eq!(
            build_bash_command_constructor(&info, &sh, ".sh")
                .get_shell_path()
                .get_path_string(),
            "/bin/bash"
        );
    }

    /// The same through the ordered map genrule passes (GenRuleBase.java:193); the expected
    /// shell is `GenRuleConfiguredTargetTest.testRequiresDarwin`'s ("/bin/bash").
    #[test]
    fn shell_path_uses_bin_bash_for_darwin_index_map() {
        let mut info: IndexMap<String, String> = IndexMap::new();
        let sh = PathFragment::create("/usr/bin/sh");
        assert_eq!(
            build_bash_command_constructor(&info, &sh, ".genrule_script.sh").get_shell_path(),
            &sh
        );
        info.insert(REQUIRES_DARWIN.to_owned(), String::new());
        assert_eq!(
            build_bash_command_constructor(&info, &sh, ".genrule_script.sh")
                .get_shell_path()
                .get_path_string(),
            "/bin/bash"
        );
    }

    #[test]
    fn max_command_length_by_os() {
        assert_eq!(max_command_length(OS::Linux), 64000);
        assert_eq!(max_command_length(OS::Windows), 8000);
    }

    #[test]
    fn bash_command_constructor_argv() {
        let c = build_bash_command_constructor(
            &BTreeMap::<String, String>::new(),
            &PathFragment::create("/bin/bash"),
            ".genrule_script.sh",
        );
        // Java: BashCommandConstructor.asExecArgv(String) = [shell, "-c", command].
        assert_eq!(
            c.as_exec_argv("echo hi"),
            vec![
                "/bin/bash".to_owned(),
                "-c".to_owned(),
                "echo hi".to_owned()
            ]
        );
    }

    // Java: the internal string of "é" (UTF-8 C3 A9) is two Latin-1 chars; of U+1F437 four; a
    // raw byte FF is one.
    #[test]
    fn java_string_length_counts_internal_string_chars() {
        let i = |s: &[u8]| blaze_util::string_encoding::latin1_decode(s).into_owned();
        assert_eq!(java_string_length("abc"), 3);
        assert_eq!(java_string_length(&i("\u{e9}".as_bytes())), 2);
        assert_eq!(java_string_length(&i("\u{1F437}".as_bytes())), 4);
        assert_eq!(java_string_length(&i(b"\xff")), 1);
    }
}
