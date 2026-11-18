//! The `BlazeDirectories` paths the loading functions read.
//!
//! PORT: Java passes `BlazeDirectories` (lib/analysis/BlazeDirectories.java) to the function
//! constructors. Its port lives in `blaze_runtime`, above this crate, so the runtime copies the
//! few paths needed here into [`LoadingDirectories`].

use blaze_cmdline::label_constants::EXTERNAL_REPOSITORY_LOCATION;
use blaze_vfs::Path;

/// The directories of one Blaze server, as the loading functions see them.
#[derive(Clone, Debug)]
pub struct LoadingDirectories {
    /// Java `BlazeDirectories.getWorkspace()`.
    pub workspace: Path,
    /// Java `BlazeDirectories.getOutputBase()`.
    pub output_base: Path,
    /// Java `BlazeDirectories.getInstallBase()`.
    pub install_base: Path,
}

impl LoadingDirectories {
    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:136 BlazeDirectories.getWorkspace
    pub fn get_workspace(&self) -> &Path {
        &self.workspace
    }

    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:163 BlazeDirectories.getOutputBase
    pub fn get_output_base(&self) -> &Path {
        &self.output_base
    }

    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:125 BlazeDirectories.getInstallBase
    pub fn get_install_base(&self) -> &Path {
        &self.install_base
    }

    /// `<output_base>/external`.
    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryUtils.java:74 RepositoryUtils.getExternalRepositoryDirectory
    pub fn get_external_repository_directory(&self) -> Path {
        self.output_base.get_relative(&EXTERNAL_REPOSITORY_LOCATION)
    }
}
