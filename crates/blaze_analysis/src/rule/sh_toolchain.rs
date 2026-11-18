//! Port of `analysis/ShToolchain.java`: the shell executable for a platform.

use std::sync::Arc;

use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::fragments::shell_configuration::ShellConfiguration;
use blaze_shell::util::os::OS;
use blaze_vfs::PathFragment;

use crate::platform::platform_info::PlatformInfo;

/// `OS.name()`, the key of `ShellConfiguration.getShellExecutable(OS)`.
fn os_name(os: OS) -> &'static str {
    match os {
        OS::Darwin => "DARWIN",
        OS::Freebsd => "FREEBSD",
        OS::Openbsd => "OPENBSD",
        OS::Linux => "LINUX",
        OS::Windows => "WINDOWS",
        OS::Unknown => "UNKNOWN",
    }
}

/// Returns the shell executable for the host platform (Java `getPathForHost`).
// Java: com/google/devtools/build/lib/analysis/ShToolchain.java:29 ShToolchain.getPathForHost
pub fn get_path_for_host(config: &Arc<BuildConfigurationValue>) -> PathFragment {
    get_path_for_platform(config, None)
}

/// Returns the shell executable for the given platform: the `--shell_executable` value if set,
/// else the default shell of the platform's OS, else the default for `OS.UNKNOWN`.
// Java: com/google/devtools/build/lib/analysis/ShToolchain.java:33 ShToolchain.getPathForPlatform
pub fn get_path_for_platform(
    config: &Arc<BuildConfigurationValue>,
    platform_info: Option<&Arc<PlatformInfo>>,
) -> PathFragment {
    if let Some(shell_configuration) = config.get_fragment::<ShellConfiguration>() {
        if let Some(d) = shell_configuration.get_options_based_default() {
            return d.clone();
        }
    }
    platform_info
        .map(|p| crate::platform::constraint_constants::get_os_from_constraints_or_host(Some(p)))
        .and_then(|os| ShellConfiguration::get_shell_executable(os_name(os)))
        .or_else(|| ShellConfiguration::get_shell_executable(os_name(OS::Unknown)))
        .expect("NoSuchElementException")
}
