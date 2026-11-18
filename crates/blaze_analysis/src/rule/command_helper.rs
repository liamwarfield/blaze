//! The parts of `analysis/CommandHelper.java` and `analysis/BashCommandConstructor.java` that
//! `ctx.actions.run_shell` uses (shell selection and the helper script for long commands), and
//! `FileWriteAction.createFile`.

use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::actions::execution_requirements::REQUIRES_DARWIN;
use blaze_actions::analysis::actions::file_write_action::FileWriteAction;
use blaze_shell::util::os::OS;
use blaze_vfs::PathFragment;

use crate::rule::rule_context::RuleContext;

/// The class for constructing command line for Bash.
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
        let script_file_name = format!(
            "{}{}",
            rule_context.get_target().get_name(),
            self.script_name_suffix
        );
        let script_file_contents = format!("#!/bin/bash\n{command}");
        create_file(
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
    let script_file_artifact = rule_context.get_package_relative_artifact(
        &PathFragment::create(file_name),
        &rule_context.get_genfiles_directory(),
    );
    rule_context.register_action(Arc::new(FileWriteAction::create_from_context(
        rule_context.get_action_owner(),
        script_file_artifact.clone(),
        contents,
        executable,
    )));
    script_file_artifact
}

/// The maximum length of a command line the execution OS accepts directly.
// Java: com/google/devtools/build/lib/analysis/CommandHelper.java:217 CommandHelper.maxCommandLength
// PORT: `setMaxCommandLengthForTesting` (a test-only override) is not ported.
pub fn max_command_length(execution_os: OS) -> usize {
    if execution_os == OS::Windows {
        8000
    } else {
        64000
    }
}

/// If `command` is too long, creates a helper shell script that runs that command, and returns
/// it. Otherwise returns `None`.
// Java: com/google/devtools/build/lib/analysis/CommandHelper.java:281 CommandHelper.commandHelperScriptMaybe
pub fn command_helper_script_maybe(
    rule_context: &RuleContext,
    command: &str,
    constructor: &BashCommandConstructor,
    execution_os: OS,
) -> Option<Artifact> {
    // Java: String.length() counts UTF-16 code units.
    if command.encode_utf16().count() <= max_command_length(execution_os) {
        None
    } else {
        Some(constructor.command_as_script(rule_context, command))
    }
}

/// Returns the path to the shell for an action with the given execution requirements.
// Java: com/google/devtools/build/lib/analysis/CommandHelper.java:324 CommandHelper.shellPath
fn shell_path(
    execution_info: &BTreeMap<String, String>,
    sh_executable: &PathFragment,
) -> PathFragment {
    // Use vanilla /bin/bash for actions running on mac machines.
    if execution_info.contains_key(REQUIRES_DARWIN) {
        PathFragment::create("/bin/bash")
    } else {
        sh_executable.clone()
    }
}

// Java: com/google/devtools/build/lib/analysis/CommandHelper.java:332 CommandHelper.buildBashCommandConstructor
pub fn build_bash_command_constructor(
    execution_info: &BTreeMap<String, String>,
    sh_executable: &PathFragment,
    script_post_fix: &str,
) -> BashCommandConstructor {
    BashCommandConstructor::new(
        shell_path(execution_info, sh_executable),
        script_post_fix.to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_path_uses_bin_bash_for_darwin() {
        let mut info = BTreeMap::new();
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

    #[test]
    fn max_command_length_by_os() {
        assert_eq!(max_command_length(OS::Linux), 64000);
        assert_eq!(max_command_length(OS::Windows), 8000);
    }
}
