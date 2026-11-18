//! Port of `analysis/config/OutputDirectories.java`, with the parts of `ArtifactRoot` and
//! `BlazeDirectories` it needs.

use blaze_cmdline::RepositoryName;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_optdefs::fragment::platform_options::PlatformOptions;
use blaze_vfs::{Path, PathFragment};

/// The parts of `BlazeDirectories` that configurations read.
///
/// PORT: `BlazeDirectories` lives in `blaze_runtime`, above this crate; the runtime builds this
/// from it (`getExecRootBase()`, `getRelativeOutputPath()`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigDirectories {
    /// `BlazeDirectories.getExecRootBase()`: `<output_base>/execroot`.
    pub exec_root_base: Path,
    /// `BlazeDirectories.getProductName()`.
    pub product_name: String,
}

impl ConfigDirectories {
    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:186 BlazeDirectories.getExecRoot
    pub fn get_exec_root(&self, workspace_name: &str) -> Path {
        self.exec_root_base.get_relative_str(workspace_name)
    }

    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:232 BlazeDirectories.getRelativeOutputPath
    pub fn get_relative_output_path(&self) -> String {
        format!("{}-out", self.product_name)
    }
}

// Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:181 ArtifactRoot.RootType
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
pub enum RootType {
    OUTPUT,
    SIBLING_MAIN_OUTPUT,
    SIBLING_EXTERNAL_OUTPUT,
}

// Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:93 ArtifactRoot
/// A root for an artifact: the derived (output) roots configurations create.
///
/// PORT: move to `blaze_actions` with the rest of `ArtifactRoot`; only derived roots exist here.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ArtifactRoot {
    root: Path,
    exec_path: PathFragment,
    root_type: RootType,
}

impl ArtifactRoot {
    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:135 ArtifactRoot.asDerivedRoot
    /// Constructs an ArtifactRoot given the output prefixes. (eg, "bin"), and (eg, "testlogs")
    /// relative to the execRoot.
    pub fn as_derived_root(exec_root: &Path, root_type: RootType, prefixes: &[&str]) -> Self {
        let mut exec_path = PathFragment::create("");
        for prefix in prefixes {
            // Tests can have empty segments here, be gentle to them.
            if !prefix.is_empty() {
                exec_path = exec_path.get_child(prefix);
            }
        }
        // Make sure that we are not creating a derived artifact under the execRoot.
        blaze_util::check_argument!(!exec_path.is_empty(), "empty execPath");
        blaze_util::check_argument!(
            !exec_path.is_absolute(),
            "execPath must be relative: {}",
            exec_path
        );
        let root = exec_root.get_relative(&exec_path);
        ArtifactRoot {
            root,
            exec_path,
            root_type,
        }
    }

    /// `getRoot().asPath()`.
    pub fn get_root_path(&self) -> &Path {
        &self.root
    }

    pub fn get_exec_path(&self) -> &PathFragment {
        &self.exec_path
    }

    pub fn get_exec_path_string(&self) -> String {
        self.exec_path.get_path_string().to_string()
    }

    pub fn get_root_type(&self) -> RootType {
        self.root_type
    }
}

// Java: com/google/devtools/build/lib/analysis/config/OutputDirectories.java:67 OutputDirectories.OutputDirectory
/// Directories in the output tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
pub enum OutputDirectory {
    BIN,
    GENFILES,
    TESTLOGS,
    OUTPUT,
}

impl OutputDirectory {
    fn name(&self) -> &'static str {
        match self {
            OutputDirectory::BIN => "bin",
            OutputDirectory::GENFILES => "genfiles",
            OutputDirectory::TESTLOGS => "testlogs",
            OutputDirectory::OUTPUT => "",
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/OutputDirectories.java:83 OutputDirectory.getRoot
    pub fn get_root(
        &self,
        output_dir_name: &str,
        directories: &ConfigDirectories,
        workspace_name: &str,
    ) -> ArtifactRoot {
        // e.g., execroot/my_workspace
        let exec_root = directories.get_exec_root(workspace_name);
        // e.g., [[execroot/my_workspace]/bazel-out/config/bin]
        ArtifactRoot::as_derived_root(
            &exec_root,
            RootType::OUTPUT,
            &[
                &directories.get_relative_output_path(),
                output_dir_name,
                self.name(),
            ],
        )
    }
}

// Java: com/google/devtools/build/lib/analysis/config/OutputDirectories.java:36 OutputDirectories
/// Logic for figuring out what base directories to place outputs generated from a given
/// configuration.
#[derive(Clone, Debug)]
pub struct OutputDirectories {
    directories: ConfigDirectories,
    mnemonic: String,
    output_directory: ArtifactRoot,
    bin_directory: ArtifactRoot,
    genfiles_directory: ArtifactRoot,
    testlogs_directory: ArtifactRoot,
    merge_genfiles_directory: bool,
    sibling_repository_layout: bool,
    exec_root: Path,
}

impl OutputDirectories {
    // Java: com/google/devtools/build/lib/analysis/config/OutputDirectories.java:107 OutputDirectories.<init>
    pub fn new(
        directories: &ConfigDirectories,
        options: &CoreOptions,
        _platform_options: Option<&PlatformOptions>,
        mnemonic: &str,
        workspace_name: &str,
        sibling_repository_layout: bool,
    ) -> Self {
        OutputDirectories {
            directories: directories.clone(),
            mnemonic: mnemonic.to_string(),
            output_directory: OutputDirectory::OUTPUT.get_root(
                mnemonic,
                directories,
                workspace_name,
            ),
            bin_directory: OutputDirectory::BIN.get_root(mnemonic, directories, workspace_name),
            genfiles_directory: OutputDirectory::GENFILES.get_root(
                mnemonic,
                directories,
                workspace_name,
            ),
            testlogs_directory: OutputDirectory::TESTLOGS.get_root(
                mnemonic,
                directories,
                workspace_name,
            ),
            merge_genfiles_directory: options.merge_genfiles_directory,
            sibling_repository_layout,
            exec_root: directories.get_exec_root(workspace_name),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/OutputDirectories.java:127 OutputDirectories.buildDerivedRoot
    fn build_derived_root(&self, name_fragment: &str, repository: &RepositoryName) -> ArtifactRoot {
        ArtifactRoot::as_derived_root(
            &self.exec_root,
            // e.g., execroot/mainRepoName/bazel-out/[repoName/]config/bin
            if repository.is_main() {
                RootType::SIBLING_MAIN_OUTPUT
            } else {
                RootType::SIBLING_EXTERNAL_OUTPUT
            },
            &[
                &self.directories.get_relative_output_path(),
                repository.get_name(),
                &self.mnemonic,
                name_fragment,
            ],
        )
    }

    // Java: com/google/devtools/build/lib/analysis/config/OutputDirectories.java:142 OutputDirectories.getOutputDirectory
    /// Returns the output directory for this build configuration.
    pub fn get_output_directory(&self, repository_name: &RepositoryName) -> ArtifactRoot {
        if self.sibling_repository_layout {
            self.build_derived_root("", repository_name)
        } else {
            self.output_directory.clone()
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/OutputDirectories.java:147 OutputDirectories.getBinDirectory
    /// Returns the bin directory for this build configuration.
    pub fn get_bin_directory(&self, repository_name: &RepositoryName) -> ArtifactRoot {
        if self.sibling_repository_layout {
            self.build_derived_root("bin", repository_name)
        } else {
            self.bin_directory.clone()
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/OutputDirectories.java:152 OutputDirectories.getGenfilesDirectory
    /// Returns the genfiles directory for this build configuration.
    pub fn get_genfiles_directory(&self, repository_name: &RepositoryName) -> ArtifactRoot {
        if self.merge_genfiles_directory {
            self.get_bin_directory(repository_name)
        } else if self.sibling_repository_layout {
            self.build_derived_root("genfiles", repository_name)
        } else {
            self.genfiles_directory.clone()
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/OutputDirectories.java:161 OutputDirectories.getTestLogsDirectory
    /// Returns the testlogs directory for this build configuration.
    pub fn get_test_logs_directory(&self, repository_name: &RepositoryName) -> ArtifactRoot {
        if self.sibling_repository_layout {
            self.build_derived_root("testlogs", repository_name)
        } else {
            self.testlogs_directory.clone()
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/OutputDirectories.java:168 OutputDirectories.getGenfilesFragment
    /// Returns a relative path to the genfiles directory at execution time.
    pub fn get_genfiles_fragment(&self, repository_name: &RepositoryName) -> PathFragment {
        self.get_genfiles_directory(repository_name)
            .get_exec_path()
            .clone()
    }

    // Java: com/google/devtools/build/lib/analysis/config/OutputDirectories.java:178 OutputDirectories.getHostPathSeparator
    pub fn get_host_path_separator(&self) -> &'static str {
        // PORT: skipped, Windows only (";").
        ":"
    }

    pub fn get_mnemonic(&self) -> &str {
        &self.mnemonic
    }

    pub fn get_output_dir_name(&self) -> &str {
        self.get_mnemonic()
    }

    pub fn merge_genfiles_directory(&self) -> bool {
        self.merge_genfiles_directory
    }

    pub fn get_directories(&self) -> &ConfigDirectories {
        &self.directories
    }
}

/// `RepositoryName.MAIN`.
pub fn main_repository() -> blaze_cmdline::RepositoryName {
    blaze_cmdline::repository_name::MAIN.clone()
}
