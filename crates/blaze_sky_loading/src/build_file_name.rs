//! Port of `BuildFileName`.
//!
//! PORT: move to blaze_packages (Java `lib/packages`); it lives here until the package crate has
//! a home for it, because PACKAGE_LOOKUP needs it now.

use std::sync::LazyLock;

use blaze_cmdline::PackageIdentifier;
use blaze_vfs::PathFragment;

/// The file (BUILD, WORKSPACE, etc.) that defines this package, referred to as the "build file".
// Java: com/google/devtools/build/lib/packages/BuildFileName.java:22 BuildFileName
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuildFileName {
    Workspace,
    WorkspaceDotBazel,
    WorkspaceDotBzlmod,
    ModuleDotBazel,
    Build,
    BuildDotBazel,
}

static FRAGMENTS: LazyLock<[PathFragment; 6]> = LazyLock::new(|| {
    [
        PathFragment::create("WORKSPACE"),
        PathFragment::create("WORKSPACE.bazel"),
        PathFragment::create("WORKSPACE.bzlmod"),
        PathFragment::create("MODULE.bazel"),
        PathFragment::create("BUILD"),
        PathFragment::create("BUILD.bazel"),
    ]
});

impl BuildFileName {
    fn index(self) -> usize {
        match self {
            BuildFileName::Workspace => 0,
            BuildFileName::WorkspaceDotBazel => 1,
            BuildFileName::WorkspaceDotBzlmod => 2,
            BuildFileName::ModuleDotBazel => 3,
            BuildFileName::Build => 4,
            BuildFileName::BuildDotBazel => 5,
        }
    }

    // Java: com/google/devtools/build/lib/packages/BuildFileName.java:67 BuildFileName.getFilenameFragment
    pub fn get_filename_fragment(self) -> &'static PathFragment {
        &FRAGMENTS[self.index()]
    }

    /// Returns a [`PathFragment`] to the build file that defines the package.
    // Java: com/google/devtools/build/lib/packages/BuildFileName.java:76 BuildFileName.getBuildFileFragment
    pub fn get_build_file_fragment(self, package_identifier: &PackageIdentifier) -> PathFragment {
        match self {
            BuildFileName::Workspace
            | BuildFileName::WorkspaceDotBazel
            | BuildFileName::WorkspaceDotBzlmod
            | BuildFileName::ModuleDotBazel => self.get_filename_fragment().clone(),
            BuildFileName::Build | BuildFileName::BuildDotBazel => package_identifier
                .get_package_fragment()
                .get_relative(self.get_filename_fragment()),
        }
    }
}
