//! Info items that Bazel's modules add in `serverInit` (`builder.addInfoItems(...)`):
//! `CommandLogModule.CommandLogInfoItem`, `BazelRepositoryModule.RepositoryCacheInfoItem` and
//! `PostGCMemoryUseRecorder.PeakMemInfoItem`.
//!
//! PORT: these live here until the modules that own them are ported.

use blaze_optdefs::misc::repository_options::RepositoryOptions;
use blaze_vfs::{Path, PathFragment};

use super::items::info_item_base;
use crate::command_environment::CommandEnvironment;
use crate::info_item::{ConfigurationSupplier, InfoItem, print};
use crate::module::AbruptExitException;

/// Info item for the command line log.
// Java: com/google/devtools/build/lib/runtime/CommandLogModule.java:119 CommandLogModule.CommandLogInfoItem
pub struct CommandLogInfoItem {
    name: String,
    description: String,
    hidden: bool,
}

impl CommandLogInfoItem {
    // Java: com/google/devtools/build/lib/runtime/CommandLogModule.java:120 CommandLogInfoItem.<init>
    pub fn new() -> Self {
        CommandLogInfoItem {
            name: "command_log".to_string(),
            description: "Location of the log containing the output from the build commands."
                .to_string(),
            hidden: false,
        }
    }
}

// Java: com/google/devtools/build/lib/runtime/CommandLogModule.java:58 CommandLogModule.getCommandLogPath
fn get_command_log_path(output_base: &Path) -> Path {
    output_base.get_relative_str("command.log")
}

impl InfoItem for CommandLogInfoItem {
    info_item_base!();

    // Java: com/google/devtools/build/lib/runtime/CommandLogModule.java:128 CommandLogInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        Ok(print(get_command_log_path(
            env.get_runtime().get_workspace().get_output_base(),
        )))
    }
}

/// Info item for the repository download cache.
// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:189 BazelRepositoryModule.RepositoryCacheInfoItem
pub struct RepositoryCacheInfoItem {
    name: String,
    description: String,
    hidden: bool,
}

// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:129 BazelRepositoryModule.DEFAULT_CACHE_LOCATION
const DEFAULT_CACHE_LOCATION: &str = "cache/repos/v1";

impl RepositoryCacheInfoItem {
    // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:192 RepositoryCacheInfoItem.<init>
    pub fn new() -> Self {
        RepositoryCacheInfoItem {
            name: "repository_cache".to_string(),
            description: "The location of the repository download cache used".to_string(),
            hidden: false,
        }
    }

    /// `repositoryCache.getPath()` as `BazelRepositoryModule.beforeCommand` sets it.
    ///
    /// PORT: Java's item holds the module's `RepositoryCache`, whose path `beforeCommand` sets
    /// from `RepositoryOptions` each command (and leaves alone when the command has no
    /// `RepositoryOptions`, so it keeps the previous command's value, initially null). Blaze has
    /// no `BazelRepositoryModule` yet and runs one command per process, so the path is computed
    /// here from the same options.
    pub(crate) fn get_repository_cache_path(env: &CommandEnvironment) -> Option<Path> {
        let repo_options = env.get_options().get_options::<RepositoryOptions>()?;
        // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:276 BazelRepositoryModule.beforeCommand
        match &repo_options.repository_cache {
            Some(path) => Self::to_path(&path.0, env),
            None => Some(
                env.get_directories()
                    .get_server_directories()
                    .get_output_user_root()
                    .get_relative_str(DEFAULT_CACHE_LOCATION),
            ),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:715 BazelRepositoryModule.toPath
    pub(crate) fn to_path(path: &PathFragment, env: &CommandEnvironment) -> Option<Path> {
        let workspace = env.get_directories().get_workspace();
        if path.is_empty() || workspace.is_none() {
            return None;
        }
        // It is important to use getWorkspace() here, not getWorkingDirectory(). Both Paths have
        // the same underlying PathFragment, but may differ in their FileSystem if the remote repo
        // contents cache is in use.
        Some(workspace.unwrap().get_relative(path))
    }
}

impl InfoItem for RepositoryCacheInfoItem {
    info_item_base!();

    // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:198 RepositoryCacheInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        match Self::get_repository_cache_path(env) {
            Some(path) => Ok(print(path)),
            None => Ok(print("null")),
        }
    }
}

/// Info item for the peak post-GC heap size.
// Java: com/google/devtools/build/lib/metrics/PostGCMemoryUseRecorder.java:245 PostGCMemoryUseRecorder.PeakMemInfoItem
pub struct PeakMemInfoItem {
    name: String,
    description: String,
    hidden: bool,
}

impl PeakMemInfoItem {
    // Java: com/google/devtools/build/lib/metrics/PostGCMemoryUseRecorder.java:246 PeakMemInfoItem.<init>
    pub fn new() -> Self {
        PeakMemInfoItem {
            name: "peak-heap-size".to_string(),
            description: "The peak amount of used memory in bytes after any full GC during the \
                          most recent invocation."
                .to_string(),
            hidden: true,
        }
    }
}

impl InfoItem for PeakMemInfoItem {
    info_item_base!();

    /// PORT: Blaze has no garbage collector; the peak heap after a full GC is the peak resident
    /// memory of the process (ruling D15), "unknown" where the system does not report it.
    // Java: com/google/devtools/build/lib/metrics/PostGCMemoryUseRecorder.java:255 PeakMemInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        _env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        Ok(match crate::process_memory::peak_resident_bytes() {
            Some(peak) => print(super::items::pretty_print_bytes(peak)),
            None => print("unknown"),
        })
    }
}
