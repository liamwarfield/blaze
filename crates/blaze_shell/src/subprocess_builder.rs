//! Port of `com.google.devtools.build.lib.shell.SubprocessBuilder`.

use std::fmt;
use std::io;
use std::sync::Arc;

use indexmap::IndexMap;
use parking_lot::RwLock;

use crate::java_file::JavaFile;
use crate::java_subprocess_factory::JavaSubprocessFactory;
use crate::subprocess::{Subprocess, SubprocessFactory};

/// What to do with an output stream of the process.
// Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:33 SubprocessBuilder.StreamAction
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SubprocessBuilderStreamAction {
    /// Redirect to a file
    Redirect,
    /// Discard.
    Discard,
    /// Stream back to the parent process using an output stream.
    Stream,
}

static DEFAULT_FACTORY: RwLock<Option<Arc<dyn SubprocessFactory>>> = RwLock::new(None);

// Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:58 SubprocessBuilder.subprocessFactoryImplementation
fn subprocess_factory_implementation() -> Arc<dyn SubprocessFactory> {
    // PORT: skipped, Windows only (WindowsSubprocessFactory).
    JavaSubprocessFactory::instance()
}

fn default_factory() -> Arc<dyn SubprocessFactory> {
    if let Some(f) = DEFAULT_FACTORY.read().as_ref() {
        return f.clone();
    }
    subprocess_factory_implementation()
}

/// A builder class that starts a subprocess.
// Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:31 SubprocessBuilder
#[derive(Clone)]
pub struct SubprocessBuilder {
    factory: Arc<dyn SubprocessFactory>,
    client_env: IndexMap<String, String>,
    argv: Vec<String>,
    env: Option<IndexMap<String, String>>,
    stdout_action: SubprocessBuilderStreamAction,
    stdout_file: Option<JavaFile>,
    stderr_action: SubprocessBuilderStreamAction,
    stderr_file: Option<JavaFile>,
    working_directory: Option<JavaFile>,
    timeout_millis: i64,
    redirect_error_stream: bool,
}

impl fmt::Debug for SubprocessBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SubprocessBuilder")
            .field("argv", &self.argv)
            .field("env", &self.env)
            .field("working_directory", &self.working_directory)
            .field("timeout_millis", &self.timeout_millis)
            .finish_non_exhaustive()
    }
}

impl SubprocessBuilder {
    /// Sets the default factory class for creating subprocesses. Passing `None` resets it to the
    /// initial state.
    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:71 SubprocessBuilder.setDefaultSubprocessFactory
    pub fn set_default_subprocess_factory(factory: Option<Arc<dyn SubprocessFactory>>) {
        *DEFAULT_FACTORY.write() = factory;
    }

    /// Creates a new subprocess builder. `client_env` is inherited by the subprocess unless
    /// [`Self::set_env`] is called with `Some`.
    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:82 SubprocessBuilder.<init>
    pub fn new(client_env: &IndexMap<String, String>) -> SubprocessBuilder {
        SubprocessBuilder::with_factory(client_env, default_factory())
    }

    /// Creates a new subprocess builder using the given factory.
    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:93 SubprocessBuilder.<init>
    pub fn with_factory(
        client_env: &IndexMap<String, String>,
        factory: Arc<dyn SubprocessFactory>,
    ) -> SubprocessBuilder {
        SubprocessBuilder {
            factory,
            client_env: client_env.clone(),
            argv: Vec::new(),
            env: None,
            stdout_action: SubprocessBuilderStreamAction::Stream,
            stdout_file: None,
            stderr_action: SubprocessBuilderStreamAction::Stream,
            stderr_file: None,
            working_directory: None,
            timeout_millis: 0,
            redirect_error_stream: false,
        }
    }

    /// Returns the complete argv, including argv0.
    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:107 SubprocessBuilder.getArgv
    pub fn get_argv(&self) -> &[String] {
        &self.argv
    }

    /// Sets the argv, including argv[0], that is, the binary to execute. Panics (Java
    /// `IllegalArgumentException`) if argv is empty, or its first element is neither an absolute
    /// path nor just a single file name.
    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:122 SubprocessBuilder.setArgv
    pub fn set_argv(&mut self, argv: Vec<String>) -> &mut Self {
        self.argv = argv;
        blaze_util::check_argument!(!self.argv.is_empty());
        // PORT: `StringEncoding.internalToPlatform` is the identity in the Latin-1 locale (see
        // `java_subprocess_factory`).
        let argv0 = JavaFile::new(&self.argv[0]);
        blaze_util::check_argument!(
            argv0.is_absolute() || argv0.get_parent().is_none(),
            "argv[0] = '{}'; it should be either absolute or just a single file name (no directory component)",
            self.argv[0]
        );
        self
    }

    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:134 SubprocessBuilder.getEnv
    pub fn get_env(&self) -> Option<&IndexMap<String, String>> {
        self.env.as_ref()
    }

    /// Sets the environment passed to the child process. If `None`, inherit the client
    /// environment passed to the constructor. The default is to inherit.
    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:143 SubprocessBuilder.setEnv
    pub fn set_env(&mut self, env: Option<IndexMap<String, String>>) -> &mut Self {
        self.env = env;
        self
    }

    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:148 SubprocessBuilder.getStdout
    pub fn get_stdout(&self) -> SubprocessBuilderStreamAction {
        self.stdout_action
    }

    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:152 SubprocessBuilder.getStdoutFile
    pub fn get_stdout_file(&self) -> Option<&JavaFile> {
        self.stdout_file.as_ref()
    }

    /// Tells the object what to do with stdout: either stream as an input stream or discard.
    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:162 SubprocessBuilder.setStdout
    pub fn set_stdout_action(&mut self, action: SubprocessBuilderStreamAction) -> &mut Self {
        if action == SubprocessBuilderStreamAction::Redirect {
            panic!("IllegalStateException");
        }
        self.stdout_action = action;
        self.stdout_file = None;
        self
    }

    /// Sets the file stdout is appended to.
    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:176 SubprocessBuilder.setStdout
    pub fn set_stdout_file(&mut self, file: JavaFile) -> &mut Self {
        self.stdout_action = SubprocessBuilderStreamAction::Redirect;
        self.stdout_file = Some(file);
        self
    }

    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:183 SubprocessBuilder.setTimeoutMillis
    pub fn set_timeout_millis(&mut self, timeout_millis: i64) -> &mut Self {
        self.timeout_millis = timeout_millis;
        self
    }

    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:188 SubprocessBuilder.getTimeoutMillis
    pub fn get_timeout_millis(&self) -> i64 {
        self.timeout_millis
    }

    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:192 SubprocessBuilder.getStderr
    pub fn get_stderr(&self) -> SubprocessBuilderStreamAction {
        self.stderr_action
    }

    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:196 SubprocessBuilder.getStderrFile
    pub fn get_stderr_file(&self) -> Option<&JavaFile> {
        self.stderr_file.as_ref()
    }

    /// Sets the file stderr is appended to. When `redirectErrorStream` is set, this has no effect.
    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:206 SubprocessBuilder.setStderr
    pub fn set_stderr_file(&mut self, file: JavaFile) -> &mut Self {
        self.stderr_action = SubprocessBuilderStreamAction::Redirect;
        self.stderr_file = Some(file);
        self
    }

    /// Tells whether this process builder merges standard error and standard output.
    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:217 SubprocessBuilder.redirectErrorStream
    pub fn redirect_error_stream(&self) -> bool {
        self.redirect_error_stream
    }

    /// Sets whether this process builder merges standard error and standard output.
    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:231 SubprocessBuilder.redirectErrorStream
    pub fn set_redirect_error_stream(&mut self, redirect_error_stream: bool) -> &mut Self {
        self.redirect_error_stream = redirect_error_stream;
        self
    }

    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:236 SubprocessBuilder.getWorkingDirectory
    pub fn get_working_directory(&self) -> Option<&JavaFile> {
        self.working_directory.as_ref()
    }

    /// Sets the current working directory. If `None`, it will be that of this process.
    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:242 SubprocessBuilder.setWorkingDirectory
    pub fn set_working_directory(&mut self, working_directory: Option<JavaFile>) -> &mut Self {
        self.working_directory = working_directory;
        self
    }

    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:247 SubprocessBuilder.getClientEnv
    pub fn get_client_env(&self) -> &IndexMap<String, String> {
        &self.client_env
    }

    // Java: com/google/devtools/build/lib/shell/SubprocessBuilder.java:251 SubprocessBuilder.start
    pub fn start(&self) -> io::Result<Box<dyn Subprocess>> {
        self.factory.create(self)
    }
}
