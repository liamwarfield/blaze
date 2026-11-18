//! Port of `com.google.devtools.build.lib.util.CommandBuilder`.

use indexmap::IndexMap;

use crate::command::Command;
use crate::java_file::JavaFile;
use crate::util::os::OS;

const SHELLS: [&str; 2] = ["/bin/sh", "/bin/bash"];

/// Implements OS aware [`Command`] builder. At this point only Linux, Mac and Windows XP are
/// supported.
///
/// Builder will also apply heuristic to identify trivial cases where unix-like command lines
/// could be automatically converted into the Windows-compatible form.
// Java: com/google/devtools/build/lib/util/CommandBuilder.java:47 CommandBuilder
#[derive(Clone, Debug)]
pub struct CommandBuilder {
    system: OS,
    client_env: IndexMap<String, String>,
    argv: Vec<String>,
    // PORT: java-hash-order. Java uses a HashMap; its order only reaches the child's environ
    // block (whose order the JDK does not define either) and describeCommand sorts it.
    env: IndexMap<String, String>,
    working_dir: Option<JavaFile>,
    use_shell: bool,
}

impl CommandBuilder {
    // Java: com/google/devtools/build/lib/util/CommandBuilder.java:60 CommandBuilder.<init>
    pub fn new(client_env: &IndexMap<String, String>) -> CommandBuilder {
        CommandBuilder::with_os(OS::get_current(), client_env)
    }

    // Java: com/google/devtools/build/lib/util/CommandBuilder.java:65 CommandBuilder.<init>
    pub fn with_os(system: OS, client_env: &IndexMap<String, String>) -> CommandBuilder {
        CommandBuilder {
            system,
            client_env: client_env.clone(),
            argv: Vec::new(),
            env: IndexMap::new(),
            working_dir: None,
            use_shell: false,
        }
    }

    // Java: com/google/devtools/build/lib/util/CommandBuilder.java:71 CommandBuilder.addArg
    pub fn add_arg(&mut self, arg: &str) -> &mut Self {
        self.argv.push(arg.to_string());
        self
    }

    // Java: com/google/devtools/build/lib/util/CommandBuilder.java:78 CommandBuilder.addArgs
    pub fn add_args<S: AsRef<str>>(&mut self, args: &[S]) -> &mut Self {
        self.argv
            .extend(args.iter().map(|a| a.as_ref().to_string()));
        self
    }

    // Java: com/google/devtools/build/lib/util/CommandBuilder.java:89 CommandBuilder.addEnv
    pub fn add_env(&mut self, env: &IndexMap<String, String>) -> &mut Self {
        for (k, v) in env {
            self.env.insert(k.clone(), v.clone());
        }
        self
    }

    // Java: com/google/devtools/build/lib/util/CommandBuilder.java:96 CommandBuilder.emptyEnv
    pub fn empty_env(&mut self) -> &mut Self {
        self.env.clear();
        self
    }

    // Java: com/google/devtools/build/lib/util/CommandBuilder.java:102 CommandBuilder.setEnv
    pub fn set_env(&mut self, env: &IndexMap<String, String>) -> &mut Self {
        self.empty_env();
        self.add_env(env);
        self
    }

    // Java: com/google/devtools/build/lib/util/CommandBuilder.java:109 CommandBuilder.setWorkingDir
    pub fn set_working_dir(&mut self, path: &blaze_vfs::Path) -> &mut Self {
        // Path.getPathFile() is new File(getPathString()).
        self.working_dir = Some(JavaFile::new(path.get_path_string()));
        self
    }

    // Java: com/google/devtools/build/lib/util/CommandBuilder.java:116 CommandBuilder.useTempDir
    pub fn use_temp_dir(&mut self) -> &mut Self {
        // java.io.tmpdir: the JDK's Linux default is "/tmp" (it ignores $TMPDIR), and the Bazel
        // client does not set it.
        self.working_dir = Some(JavaFile::new("/tmp"));
        self
    }

    // Java: com/google/devtools/build/lib/util/CommandBuilder.java:122 CommandBuilder.useShell
    pub fn use_shell(&mut self, use_shell: bool) -> &mut Self {
        self.use_shell = use_shell;
        self
    }

    // Java: com/google/devtools/build/lib/util/CommandBuilder.java:127 CommandBuilder.argvStartsWithSh
    fn argv_starts_with_sh(&self) -> bool {
        self.argv.len() >= 2 && SHELLS.contains(&self.argv[0].as_str()) && self.argv[1] == "-c"
    }

    // Java: com/google/devtools/build/lib/util/CommandBuilder.java:131 CommandBuilder.transformArgvForLinux
    fn transform_argv_for_linux(&self) -> Vec<String> {
        // If command line already starts with "/bin/sh -c", ignore useShell attribute.
        if self.use_shell && !self.argv_starts_with_sh() {
            // c.g.io.base.shell.Shell.shellify() actually concatenates argv into the
            // space-separated string here. Not sure why, but we will do the same.
            return vec!["/bin/sh".to_string(), "-c".to_string(), self.argv.join(" ")];
        }
        self.argv.clone()
    }

    // Java: com/google/devtools/build/lib/util/CommandBuilder.java:141 CommandBuilder.transformArgvForWindows
    fn transform_argv_for_windows(&mut self) -> Vec<String> {
        // Heuristic: replace "/bin/sh -c" with something more appropriate for Windows.
        let mut modified_argv = if self.argv_starts_with_sh() {
            self.use_shell = true;
            self.argv[2..].to_vec()
        } else {
            self.argv.clone()
        };

        if !modified_argv.is_empty() {
            // args can contain whitespace, so figure out the first word
            let argv0 = modified_argv[0].clone();
            let command = argv0.split([' ', '\t']).next().unwrap();

            // Automatically enable CMD.EXE use if we are executing something else besides "*.exe"
            // file. When use CMD.EXE to invoke a bat/cmd file, the file path must have '\'
            // instead of '/'
            if !command.to_lowercase().ends_with(".exe") {
                self.use_shell = true;
                modified_argv[0] = argv0.replace('/', "\\");
            }
        } else {
            // This is degenerate "/bin/sh -c" case. We ensure that Windows behavior is identical
            // to the Linux - call shell that will do nothing.
            self.use_shell = true;
        }
        if self.use_shell {
            // /S - strip first and last quotes and execute everything else as is.
            // /E:ON - enable extended command set.
            // /V:ON - enable delayed variable expansion
            // /D - ignore AutoRun registry entries.
            // /C - execute command. This must be the last option before the command itself.
            vec![
                "CMD.EXE".to_string(),
                "/S".to_string(),
                "/E:ON".to_string(),
                "/V:ON".to_string(),
                "/D".to_string(),
                "/C".to_string(),
                modified_argv.join(" "),
            ]
        } else {
            modified_argv
        }
    }

    // Java: com/google/devtools/build/lib/util/CommandBuilder.java:180 CommandBuilder.build
    pub fn build(&mut self) -> Command {
        blaze_util::check_state!(self.system != OS::Unknown, "Unidentified operating system");
        let Some(working_dir) = self.working_dir.clone() else {
            panic!("NullPointerException: Working directory must be set");
        };
        blaze_util::check_state!(!self.argv.is_empty(), "At least one argument is expected");

        let argv = if self.system == OS::Windows {
            self.transform_argv_for_windows()
        } else {
            self.transform_argv_for_linux()
        };
        Command::with_env(
            argv,
            Some(self.env.clone()),
            Some(working_dir),
            &self.client_env,
        )
    }
}
