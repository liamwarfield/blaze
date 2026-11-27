//! Port of `com.google.devtools.build.lib.util.ScriptUtil`.
// PORT: move to blaze_util (it is a lib/util class; blaze_util does not have it yet).

use indexmap::IndexMap;

use crate::util::os::OS;
use crate::util::shell_escaper::ShellEscaper;

/// Utils for emitting scripts cross-platform.
// Java: com/google/devtools/build/lib/util/ScriptUtil.java:26 ScriptUtil
pub struct ScriptUtil;

// The Java class picks a `ScriptEmitter` by OS once; each function below tests the OS itself.
// PORT: skipped, the ScriptEmitter interface and the Windows emitter (Linux only; the OS test stays): ScriptUtil.emitBeginIsolate, ScriptUtil.emitEndIsolate
// PORT: skipped, the ScriptEmitter interface and the Windows emitter (Linux only; the OS test stays): ScriptUtil.emitChangeDirectory, ScriptUtil.emitEnvPrefix
// PORT: skipped, the ScriptEmitter interface and the Windows emitter (Linux only; the OS test stays): ScriptUtil.emitSetEnvVar, ScriptUtil.emitUnsetEnvVar
// PORT: skipped, the ScriptEmitter interface and the Windows emitter (Linux only; the OS test stays): ScriptUtil.emitCommandElement, ScriptUtil.emitExec

fn is_windows() -> bool {
    OS::get_current() == OS::Windows
}

impl ScriptUtil {
    // Java: com/google/devtools/build/lib/util/ScriptUtil.java:30 ScriptUtil.emitBeginIsolate
    // Java: com/google/devtools/build/lib/util/ScriptUtil.java:101 ScriptUtil.LinuxScriptEmitter.emitBeginIsolate
    pub(crate) fn emit_begin_isolate(message: &mut String) {
        if is_windows() {
            // TODO(bazel-team): Implement this.
        } else {
            message.push('(');
        }
    }

    // Java: com/google/devtools/build/lib/util/ScriptUtil.java:34 ScriptUtil.emitEndIsolate
    // Java: com/google/devtools/build/lib/util/ScriptUtil.java:106 ScriptUtil.LinuxScriptEmitter.emitEndIsolate
    pub(crate) fn emit_end_isolate(message: &mut String) {
        if is_windows() {
            // TODO(bazel-team): Implement this.
        } else {
            message.push(')');
        }
    }

    /// Emits command to change directories.
    // Java: com/google/devtools/build/lib/util/ScriptUtil.java:39 ScriptUtil.emitChangeDirectory
    // Java: com/google/devtools/build/lib/util/ScriptUtil.java:111 ScriptUtil.LinuxScriptEmitter.emitChangeDirectory
    pub fn emit_change_directory(message: &mut String, cwd: &str) {
        if is_windows() {
            message.push_str("cd /d ");
            message.push_str(cwd);
            message.push('\n');
        } else {
            message.push_str("cd ");
            message.push_str(&ShellEscaper::escape_string(cwd));
            message.push_str(" && \\\n  ");
        }
    }

    /// Emits the "env" prefix, setting and unsetting the provided environment variables.
    // Java: com/google/devtools/build/lib/util/ScriptUtil.java:44 ScriptUtil.emitEnvPrefix
    // Java: com/google/devtools/build/lib/util/ScriptUtil.java:116 ScriptUtil.LinuxScriptEmitter.emitEnvPrefix
    pub fn emit_env_prefix(
        message: &mut String,
        ignore_environment: bool,
        set_env: &IndexMap<String, String>,
        unset_env: Option<&[String]>,
    ) {
        if !is_windows() {
            message.push_str(if ignore_environment {
                "env - \\\n  "
            } else {
                "env \\\n  "
            });
        }

        if let Some(unset_env) = unset_env {
            let mut sorted: Vec<&String> = unset_env.iter().collect();
            sorted.sort();
            for name in sorted {
                message.push_str("  ");
                ScriptUtil::emit_unset_env_var(message, name);
            }
        }

        let mut entries: Vec<(&String, &String)> = set_env.iter().collect();
        // Stable sort by key, as Ordering.sortedCopy does.
        entries.sort_by(|a, b| a.0.cmp(b.0));
        for (name, value) in entries {
            message.push_str("  ");
            ScriptUtil::emit_set_env_var(message, name, value);
        }
    }

    // Java: com/google/devtools/build/lib/util/ScriptUtil.java:121 ScriptUtil.LinuxScriptEmitter.emitSetEnvVar
    fn emit_set_env_var(message: &mut String, name: &str, value: &str) {
        if is_windows() {
            message.push_str(&format!("SET {name}={value}\n  "));
        } else {
            message.push_str(&ShellEscaper::escape_string(name));
            message.push('=');
            message.push_str(&ShellEscaper::escape_string(value));
            message.push_str(" \\\n  ");
        }
    }

    // Java: com/google/devtools/build/lib/util/ScriptUtil.java:130 ScriptUtil.LinuxScriptEmitter.emitUnsetEnvVar
    fn emit_unset_env_var(message: &mut String, name: &str) {
        if is_windows() {
            message.push_str(&format!("SET {name}=\n  "));
        } else {
            // Only the short form of --unset is supported on macOS.
            message.push_str("-u ");
            message.push_str(&ShellEscaper::escape_string(name));
            message.push_str(" \\\n  ");
        }
    }

    /// Formats the command element and adds it to the message. `is_binary` is true if the
    /// `command_element` is the binary to be executed.
    // Java: com/google/devtools/build/lib/util/ScriptUtil.java:71 ScriptUtil.emitCommandElement
    // Java: com/google/devtools/build/lib/util/ScriptUtil.java:136 ScriptUtil.LinuxScriptEmitter.emitCommandElement
    pub fn emit_command_element(message: &mut String, command_element: &str, is_binary: bool) {
        if is_windows() {
            // Replace the forward slashes with back slashes if the `commandElement` is the binary
            // path
            if is_binary {
                message.push_str(&command_element.replace('/', "\\"));
            } else {
                message.push_str(command_element);
            }
        } else {
            message.push_str(&ShellEscaper::escape_string(command_element));
        }
    }

    /// Emits the prefix for "exec"-ing a command.
    // Java: com/google/devtools/build/lib/util/ScriptUtil.java:77 ScriptUtil.emitExec
    // Java: com/google/devtools/build/lib/util/ScriptUtil.java:141 ScriptUtil.LinuxScriptEmitter.emitExec
    pub fn emit_exec(message: &mut String) {
        if !is_windows() {
            message.push_str("exec ");
        }
    }
}
