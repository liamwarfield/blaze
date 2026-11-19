//! The fragment-related parts of `bazel/rules/BazelRuleClassProvider.java`:
//! `StrictActionEnvConfiguration` and the shell executable finder injected into
//! `ShellConfiguration` by `BAZEL_SETUP`.

use std::any::Any;
use std::sync::Arc;

use blaze_optdefs::fragment::shell_configuration::ShellConfigurationOptions;
use blaze_vfs::PathFragment;

use crate::build_options::BuildOptions;
use crate::fragment::{Fragment, FragmentClass, FragmentClassInfo, InvalidConfigurationException};
use crate::fragment_options_class::STRICT_ACTION_ENV_OPTIONS;

// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:90 BazelRuleClassProvider.FALLBACK_SHELL
const FALLBACK_SHELL: &str = "/bin/bash";

// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:93 BazelRuleClassProvider.SHELL_EXECUTABLES
/// `ImmutableMap<OS, PathFragment>`, keyed by `OS.name()`.
// PORT: the `OS` enum lives in blaze_shell (not a dependency of this crate); the map is keyed by
// the enum constant's name, in Java's insertion order.
pub const SHELL_EXECUTABLES: &[(&str, &str)] = &[
    ("WINDOWS", "c:/msys64/usr/bin/bash.exe"),
    ("FREEBSD", "/usr/local/bin/bash"),
    ("OPENBSD", "/usr/local/bin/bash"),
    ("LINUX", "/bin/bash"),
    ("DARWIN", "/bin/bash"),
    ("UNKNOWN", FALLBACK_SHELL),
];

// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:110 BazelRuleClassProvider.StrictActionEnvConfiguration
/// `BuildConfigurationFunction` constructs `BuildOptions` out of the options required by the
/// registered fragments. We create and register this fragment exclusively to ensure
/// `StrictActionEnvOptions` is always available.
#[derive(Debug)]
pub struct BazelRuleClassProviderStrictActionEnvConfiguration;

/// `BazelRuleClassProvider.StrictActionEnvConfiguration.class`.
pub static STRICT_ACTION_ENV_CONFIGURATION: FragmentClassInfo = FragmentClassInfo {
    java_name: "com.google.devtools.build.lib.bazel.rules.BazelRuleClassProvider$StrictActionEnvConfiguration",
    starlark_name: None,
    required_options: || vec![&STRICT_ACTION_ENV_OPTIONS],
    requires_starlark_options: false,
    constructor: create,
};

fn create(options: &BuildOptions) -> Result<Arc<dyn Fragment>, InvalidConfigurationException> {
    let fragment: Arc<dyn Fragment> = Arc::new(
        BazelRuleClassProviderStrictActionEnvConfiguration::new(options),
    );
    Ok(fragment)
}

impl BazelRuleClassProviderStrictActionEnvConfiguration {
    // Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:111 BazelRuleClassProvider.StrictActionEnvConfiguration.<init>
    pub fn new(_build_options: &BuildOptions) -> Self {
        BazelRuleClassProviderStrictActionEnvConfiguration
    }
}

impl Fragment for BazelRuleClassProviderStrictActionEnvConfiguration {
    fn fragment_class(&self) -> FragmentClass {
        &STRICT_ACTION_ENV_CONFIGURATION
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:116 BazelRuleClassProvider.getDefaultPathFromOptions
/// The shell from `--shell_executable`, else from the `BAZEL_SH` environment variable of the
/// server, else `None`.
pub fn get_default_path_from_options(options: &ShellConfigurationOptions) -> Option<PathFragment> {
    if let Some(shell_executable) = &options.shell_executable {
        return Some(shell_executable.get().clone());
    }

    // Honor BAZEL_SH env variable for backwards compatibility.
    // Java reads System.getenv (the server's environment): a platform string, which is the
    // internal string of its bytes in the server's Latin-1 locale.
    if let Some(path) = std::env::var_os("BAZEL_SH") {
        return Some(PathFragment::create(
            &blaze_util::string_encoding::platform_to_internal(&path),
        ));
    }
    None
}

// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:130 BazelRuleClassProvider.getShellExecutableForOs
/// `os` is the `OS` enum constant's name (see [`SHELL_EXECUTABLES`]).
pub fn get_shell_executable_for_os(os: &str, options: &ShellConfigurationOptions) -> PathFragment {
    // TODO(ulfjack): instead of using the OS Bazel runs on, we need to use the exec platform,
    // which may be different for remote execution. For now, this can be overridden with
    // --shell_executable, so at least there's a workaround.
    match get_default_path_from_options(options) {
        Some(p) => p,
        None => PathFragment::create(
            SHELL_EXECUTABLES
                .iter()
                .find(|(k, _)| *k == os)
                .map(|(_, v)| *v)
                .unwrap_or(FALLBACK_SHELL),
        ),
    }
}
