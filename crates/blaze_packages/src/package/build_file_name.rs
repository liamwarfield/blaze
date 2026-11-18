//! Port of `packages/BuildFileName.java`.

use blaze_cmdline::PackageIdentifier;
use blaze_vfs::PathFragment;

/// The names a package's build file may have (Java `BuildFileName`).
// Java: com/google/devtools/build/lib/packages/BuildFileName.java:20 BuildFileName
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuildFileName {
    Workspace,
    WorkspaceDotBazel,
    WorkspaceDotBzlmod,
    ModuleDotBazel,
    Build,
    BuildDotBazel,
}

impl BuildFileName {
    /// The file name (Java constructor argument).
    pub fn file_name(self) -> &'static str {
        match self {
            BuildFileName::Workspace => "WORKSPACE",
            BuildFileName::WorkspaceDotBazel => "WORKSPACE.bazel",
            BuildFileName::WorkspaceDotBzlmod => "WORKSPACE.bzlmod",
            BuildFileName::ModuleDotBazel => "MODULE.bazel",
            BuildFileName::Build => "BUILD",
            BuildFileName::BuildDotBazel => "BUILD.bazel",
        }
    }

    // Java: com/google/devtools/build/lib/packages/BuildFileName.java:64 BuildFileName.getFilenameFragment
    pub fn get_filename_fragment(self) -> PathFragment {
        PathFragment::create(self.file_name())
    }

    /// The path of the build file relative to the package's root: the package's source root
    /// (Java `getPackageFragment()` for WORKSPACE-like files) joined with the file name.
    // Java: com/google/devtools/build/lib/packages/BuildFileName.java:73 BuildFileName.getBuildFileFragment
    pub fn get_build_file_fragment(self, package_identifier: &PackageIdentifier) -> PathFragment {
        match self {
            // Java: com/google/devtools/build/lib/packages/BuildFileName.java:23 BuildFileName.WORKSPACE.getBuildFileFragment
            BuildFileName::Workspace
            | BuildFileName::WorkspaceDotBazel
            | BuildFileName::WorkspaceDotBzlmod
            | BuildFileName::ModuleDotBazel => self.get_filename_fragment(),
            // Java: com/google/devtools/build/lib/packages/BuildFileName.java:47 BuildFileName.BUILD.getBuildFileFragment
            BuildFileName::Build | BuildFileName::BuildDotBazel => package_identifier
                .get_package_fragment()
                .get_relative(&self.get_filename_fragment()),
        }
    }
}
