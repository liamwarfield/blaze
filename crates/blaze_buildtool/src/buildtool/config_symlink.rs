//! Port of `buildtool/ConfigSymlink.java` and the interface it implements,
//! `analysis/config/SymlinkDefinition.java`.

use std::sync::Arc;

use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::output_directories::ArtifactRoot;
use blaze_cmdline::RepositoryName;
use blaze_optdefs::command::build_request_options::BuildRequestOptions;
use blaze_vfs::Path;
use indexmap::IndexSet;

/// Represents a single kind of convenience symlink (`bazel-bin`, etc.).
///
/// PORT: Java declares this interface in `analysis/config`; nothing else in Blaze uses it yet, so
/// it lives next to its only implementations. `Set<BuildConfigurationValue>` is a slice of
/// distinct configurations in the caller's order.
// Java: com/google/devtools/build/lib/analysis/config/SymlinkDefinition.java:23 SymlinkDefinition
pub trait SymlinkDefinition: Send + Sync {
    /// Returns the name for this symlink in the workspace.
    ///
    /// Note that this is independent of the target configuration(s) that may help determine the
    /// symlink's destination.
    // Java: com/google/devtools/build/lib/analysis/config/SymlinkDefinition.java:31 SymlinkDefinition.getLinkName
    fn get_link_name(&self, symlink_prefix: &str, workspace_base_name: &str) -> String;

    /// Returns a set of candidate destination paths for the symlink.
    ///
    /// The symlink should only be created if there is exactly one candidate. Zero candidates is a
    /// no-op, and more than one candidate means a warning about ambiguous symlink destinations
    /// should be emitted.
    // Java: com/google/devtools/build/lib/analysis/config/SymlinkDefinition.java:49 SymlinkDefinition.getLinkPaths
    fn get_link_paths(
        &self,
        build_request_options: &BuildRequestOptions,
        target_configs: &[Arc<BuildConfigurationValue>],
        repository_name: &RepositoryName,
        output_path: &Path,
        exec_root: &Path,
    ) -> IndexSet<Path>;
}

/// `ConfigSymlink.ConfigPathGetter`.
// Java: com/google/devtools/build/lib/buildtool/ConfigSymlink.java:30 ConfigSymlink.ConfigPathGetter
pub type ConfigPathGetter = fn(&BuildConfigurationValue, &RepositoryName) -> ArtifactRoot;

/// Base class for symlinks to output roots. Used only by `OutputDirectoryLinksUtils`.
// Java: com/google/devtools/build/lib/buildtool/ConfigSymlink.java:28 ConfigSymlink
pub struct ConfigSymlink {
    suffix: String,
    config_to_root: ConfigPathGetter,
}

impl ConfigSymlink {
    // Java: com/google/devtools/build/lib/buildtool/ConfigSymlink.java:37 ConfigSymlink.<init>
    pub fn new(suffix: &str, config_to_root: ConfigPathGetter) -> ConfigSymlink {
        ConfigSymlink {
            suffix: suffix.to_string(),
            config_to_root,
        }
    }
}

impl SymlinkDefinition for ConfigSymlink {
    // Java: com/google/devtools/build/lib/buildtool/ConfigSymlink.java:43 ConfigSymlink.getLinkName
    fn get_link_name(&self, symlink_prefix: &str, _workspace_base_name: &str) -> String {
        format!("{}{}", symlink_prefix, self.suffix)
    }

    // Java: com/google/devtools/build/lib/buildtool/ConfigSymlink.java:48 ConfigSymlink.getLinkPaths
    fn get_link_paths(
        &self,
        _build_request_options: &BuildRequestOptions,
        target_configs: &[Arc<BuildConfigurationValue>],
        repository_name: &RepositoryName,
        _output_path: &Path,
        _exec_root: &Path,
    ) -> IndexSet<Path> {
        target_configs
            .iter()
            .map(|config| {
                (self.config_to_root)(config, repository_name)
                    .get_root_path()
                    .clone()
            })
            .collect()
    }
}
