//! `ActionEnvironment` (the configuration-level part) and `BazelRuleClassProvider.SHELL_ACTION_ENV`.
//!
//! PORT: move `ActionEnvironment` to `blaze_actions` when it exists.

use std::collections::{BTreeMap, BTreeSet};

use blaze_optdefs::fragment::bazel_rule_class_provider::BazelRuleClassProviderStrictActionEnvOptions;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_optdefs::fragment::shell_configuration::ShellConfigurationOptions;
use blaze_options::EnvVar;
use blaze_vfs::PathFragment;

use crate::build_options::BuildOptions;

// Java: com/google/devtools/build/lib/actions/ActionEnvironment.java:48 ActionEnvironment
/// Environment variables for build or test actions: fixed values and names inherited from the
/// client environment.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ActionEnvironment {
    fixed_env: BTreeMap<String, String>,
    inherited_env: BTreeSet<String>,
}

impl ActionEnvironment {
    // Java: com/google/devtools/build/lib/actions/ActionEnvironment.java:50 ActionEnvironment.EMPTY
    pub fn empty() -> Self {
        ActionEnvironment::default()
    }

    // Java: com/google/devtools/build/lib/actions/ActionEnvironment.java:80 ActionEnvironment.split
    /// Splits the given map into a map of variables with a fixed value, and a set of variables
    /// that should be inherited, the latter of which are identified by having a `null` value in
    /// the given map.
    pub fn split(env: &BTreeMap<String, Option<String>>) -> Self {
        let mut fixed_env = BTreeMap::new();
        let mut inherited_env = BTreeSet::new();
        for (k, v) in env {
            match v {
                Some(v) => {
                    fixed_env.insert(k.clone(), v.clone());
                }
                None => {
                    inherited_env.insert(k.clone());
                }
            }
        }
        ActionEnvironment {
            fixed_env,
            inherited_env,
        }
    }

    pub fn get_fixed_env(&self) -> &BTreeMap<String, String> {
        &self.fixed_env
    }

    pub fn get_inherited_env(&self) -> &BTreeSet<String> {
        &self.inherited_env
    }
}

const FALLBACK_SHELL: &str = "/bin/bash";

// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:116 BazelRuleClassProvider.getDefaultPathFromOptions
/// PORT: `System.getenv("BAZEL_SH")` is the server's environment; Blaze runs in-process, so it is
/// the process environment.
pub fn get_default_path_from_options(options: &ShellConfigurationOptions) -> Option<PathFragment> {
    if let Some(shell_executable) = &options.shell_executable {
        return Some(shell_executable.0.clone());
    }
    // Honor BAZEL_SH env variable for backwards compatibility.
    std::env::var("BAZEL_SH")
        .ok()
        .map(|path| PathFragment::create(&path))
}

// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:130 BazelRuleClassProvider.getShellExecutableForOs
pub fn get_shell_executable_for_os(options: &ShellConfigurationOptions) -> PathFragment {
    // PORT: Linux only; SHELL_EXECUTABLES maps LINUX to /bin/bash.
    get_default_path_from_options(options).unwrap_or_else(|| PathFragment::create(FALLBACK_SHELL))
}

// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:163 BazelRuleClassProvider.pathOrDefault
pub fn path_or_default(_path: Option<&str>, _sh: Option<&PathFragment>) -> String {
    // PORT: skipped, Windows only (MSYS root computation).
    "/bin:/usr/bin:/usr/local/bin".to_string()
}

// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:139 BazelRuleClassProvider.SHELL_ACTION_ENV
pub fn shell_action_env(options: &BuildOptions) -> ActionEnvironment {
    if options.has_no_config() {
        return ActionEnvironment::empty();
    }
    let strict_action_env = options
        .get::<BazelRuleClassProviderStrictActionEnvOptions>()
        .expect("StrictActionEnvOptions")
        .use_strict_action_env;
    let shell_executable = get_shell_executable_for_os(
        options
            .get::<ShellConfigurationOptions>()
            .expect("ShellConfiguration.Options"),
    );

    let mut env: BTreeMap<String, Option<String>> = BTreeMap::new();

    // All entries in the builder that have a value of null inherit the value from the client
    // environment, which is only known at execution time - we don't want to bake the client env
    // into the configuration since any change to the configuration requires rerunning the full
    // analysis phase.
    if !strict_action_env {
        env.insert("LD_LIBRARY_PATH".into(), None);
    }

    if strict_action_env {
        env.insert(
            "PATH".into(),
            Some(path_or_default(None, Some(&shell_executable))),
        );
    } else {
        // The previous implementation used System.getenv (which uses the server's environment),
        // and fell back to a hard-coded "/bin:/usr/bin" if PATH was not set.
        env.insert("PATH".into(), None);
    }

    // Shell environment variables specified via options take precedence over the ones inherited
    // from the fragments. In the long run, these fragments will be replaced by appropriate
    // default rc files anyway.
    let core = options.get::<CoreOptions>().expect("CoreOptions");
    for env_var in &core.action_environment {
        match env_var {
            EnvVar::Set { name, value } => {
                env.insert(name.clone(), Some(value.clone()));
            }
            EnvVar::Inherit { name } => {
                env.insert(name.clone(), None);
            }
            EnvVar::Unset { name } => {
                env.remove(name);
            }
        }
    }

    if !crate::build_configuration_value::runfiles_enabled(core) {
        // Setting this environment variable is for telling the binary running in a Bazel action
        // when to use runfiles library or runfiles tree.
        env.insert("RUNFILES_MANIFEST_ONLY".into(), Some("1".into()));
    }

    ActionEnvironment::split(&env)
}
