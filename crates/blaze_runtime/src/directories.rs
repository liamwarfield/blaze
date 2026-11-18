//! `ServerDirectories` and `BlazeDirectories`.
//! PORT: move to blaze_config (06 §5.1 puts them in `blaze_config`, which does not exist yet).

use blaze_vfs::{Path, PathFragment};

/// Represents the relevant directories for the server: the location where the installation
/// is unpacked to, the location of the output base and the output user root.
// Java: com/google/devtools/build/lib/analysis/ServerDirectories.java:32 ServerDirectories
#[derive(Clone, Debug)]
pub struct ServerDirectories {
    install_base: Path,
    /// Lowercase hex, validated to be 128 bits (Java keeps a `HashCode`).
    install_md5: Option<String>,
    output_base: Path,
    output_user_root: Path,
    exec_root_base: Path,
}

impl ServerDirectories {
    // Java: com/google/devtools/build/lib/analysis/ServerDirectories.java:33 ServerDirectories.EXECROOT
    pub const EXECROOT: &'static str = "execroot";

    /// PORT: `virtualSourceRoot` is always null in Bazel (no module provides a virtual source
    /// root), so it is not carried.
    // Java: com/google/devtools/build/lib/analysis/ServerDirectories.java:54 ServerDirectories.<init>
    pub fn new(
        install_base: Path,
        output_base: Path,
        output_user_root: Path,
        exec_root_base: Path,
        install_md5: Option<&str>,
    ) -> Self {
        ServerDirectories {
            install_base,
            install_md5: Self::to_md5_hash_code(install_md5),
            output_base,
            output_user_root,
            exec_root_base,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/ServerDirectories.java:62 ServerDirectories.toMD5HashCode
    fn to_md5_hash_code(install_md5: Option<&str>) -> Option<String> {
        let install_md5 = install_md5.filter(|s| !s.is_empty())?;
        // HashCode.fromString: even number of hex digits, at least 2.
        let valid = install_md5.len() >= 2
            && install_md5.len() % 2 == 0
            && install_md5.bytes().all(|b| b.is_ascii_hexdigit());
        blaze_util::check_argument!(valid, "Illegal hexadecimal character in: {}", install_md5);
        let bits = install_md5.len() * 4;
        blaze_util::check_argument!(
            bits == 128,
            "Hash '{}' has {} bits",
            install_md5.to_lowercase(),
            bits
        );
        Some(install_md5.to_lowercase())
    }

    // Java: com/google/devtools/build/lib/analysis/ServerDirectories.java:93 ServerDirectories.getInstallBase
    pub fn get_install_base(&self) -> &Path {
        &self.install_base
    }

    /// Returns the installed md5 of the server (`HashCode.toString()`, lowercase hex).
    // Java: com/google/devtools/build/lib/analysis/ServerDirectories.java:101 ServerDirectories.getInstallMD5
    pub fn get_install_md5(&self) -> Option<&str> {
        self.install_md5.as_deref()
    }

    // Java: com/google/devtools/build/lib/analysis/ServerDirectories.java:109 ServerDirectories.getOutputBase
    pub fn get_output_base(&self) -> &Path {
        &self.output_base
    }

    // Java: com/google/devtools/build/lib/analysis/ServerDirectories.java:116 ServerDirectories.getOutputUserRoot
    pub fn get_output_user_root(&self) -> &Path {
        &self.output_user_root
    }

    // Java: com/google/devtools/build/lib/analysis/ServerDirectories.java:126 ServerDirectories.getExecRootBase
    pub fn get_exec_root_base(&self) -> &Path {
        &self.exec_root_base
    }
}

/// Encapsulation of all of the interesting top-level directories in any Blaze application.
// Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:71 BlazeDirectories
#[derive(Clone, Debug)]
pub struct BlazeDirectories {
    server_directories: ServerDirectories,
    workspace: Option<Path>,
    default_system_javabase: Option<Path>,
    blaze_exec_root: Option<Path>,
    blaze_output_path: Option<Path>,
    local_output_path: Path,
    product_name: String,
}

impl BlazeDirectories {
    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:72 BlazeDirectories.DEFAULT_EXEC_ROOT
    const DEFAULT_EXEC_ROOT: &'static str = "default-exec-root";

    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:90 BlazeDirectories.<init>
    pub fn new(
        server_directories: ServerDirectories,
        workspace: Option<Path>,
        default_system_javabase: Option<Path>,
        product_name: &str,
    ) -> Self {
        let output_base = server_directories.get_output_base().clone();
        let (blaze_exec_root, blaze_output_path) = if product_name.eq_ignore_ascii_case("blaze") {
            let use_default_exec_root_name = workspace
                .as_ref()
                .is_none_or(|w| w.get_parent_directory().is_none());
            let exec_root = if use_default_exec_root_name {
                output_base
                    .get_child(ServerDirectories::EXECROOT)
                    .get_child(Self::DEFAULT_EXEC_ROOT)
            } else {
                output_base
                    .get_child(ServerDirectories::EXECROOT)
                    .get_child(workspace.as_ref().unwrap().get_base_name())
            };
            let output_path =
                exec_root.get_relative_str(&Self::get_relative_output_path_of(product_name));
            (Some(exec_root), Some(output_path))
        } else {
            (None, None)
        };
        let local_output_path =
            output_base.get_relative_str(&Self::get_relative_output_path_of(product_name));
        BlazeDirectories {
            server_directories,
            workspace,
            default_system_javabase,
            blaze_exec_root,
            blaze_output_path,
            local_output_path,
            product_name: product_name.to_string(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:120 BlazeDirectories.getServerDirectories
    pub fn get_server_directories(&self) -> &ServerDirectories {
        &self.server_directories
    }

    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:125 BlazeDirectories.getInstallBase
    pub fn get_install_base(&self) -> &Path {
        self.server_directories.get_install_base()
    }

    /// Returns the base of the workspace (`null` outside a workspace).
    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:136 BlazeDirectories.getWorkspace
    pub fn get_workspace(&self) -> Option<Path> {
        // Make sure to use the same file system as exec root.
        self.workspace.as_ref().map(|w| {
            Path::create(
                w.as_fragment().clone(),
                self.get_exec_root_base().get_file_system(),
            )
        })
    }

    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:145 BlazeDirectories.getWorkingDirectory
    pub fn get_working_directory(&self) -> Option<&Path> {
        self.workspace.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:150 BlazeDirectories.getLocalJavabase
    pub fn get_local_javabase(&self) -> Option<&Path> {
        self.default_system_javabase.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:155 BlazeDirectories.inWorkspace
    pub fn in_workspace(&self) -> bool {
        self.workspace.is_some()
    }

    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:163 BlazeDirectories.getOutputBase
    pub fn get_output_base(&self) -> &Path {
        self.server_directories.get_output_base()
    }

    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:139 BlazeDirectories.getExecRootBase
    pub fn get_exec_root_base(&self) -> &Path {
        self.server_directories.get_exec_root_base()
    }

    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:177 BlazeDirectories.getBlazeExecRoot
    pub fn get_blaze_exec_root(&self) -> &Path {
        self.blaze_exec_root
            .as_ref()
            .expect("No Blaze exec root in Bazel")
    }

    /// Returns the execution root of the main repository (`<execroot base>/<workspace name>`).
    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:186 BlazeDirectories.getExecRoot
    pub fn get_exec_root(&self, workspace_name: &str) -> Path {
        self.get_exec_root_base().get_relative_str(workspace_name)
    }

    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:196 BlazeDirectories.getBlazeOutputPath
    pub fn get_blaze_output_path(&self) -> &Path {
        self.blaze_output_path
            .as_ref()
            .expect("No Blaze output path in Bazel")
    }

    /// Returns the output path of the main repository (`<exec root>/bazel-out`).
    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:201 BlazeDirectories.getOutputPath
    pub fn get_output_path(&self, workspace_name: &str) -> Path {
        self.get_exec_root(workspace_name)
            .get_relative_str(&self.get_relative_output_path())
    }

    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:206 BlazeDirectories.getLocalOutputPath
    pub fn get_local_output_path(&self) -> &Path {
        &self.local_output_path
    }

    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:214 BlazeDirectories.getActionTempsDirectory
    pub fn get_action_temps_directory(&self, exec_root: &Path) -> Path {
        exec_root
            .get_relative_str(&self.get_relative_output_path())
            .get_relative(&PathFragment::create("_tmp/actions"))
    }

    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:219 BlazeDirectories.getEmbeddedBinariesRoot
    pub fn get_embedded_binaries_root(&self) -> &Path {
        self.get_install_base()
    }

    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:239 BlazeDirectories.getInstallMD5
    pub fn get_install_md5(&self) -> Option<&str> {
        self.server_directories.get_install_md5()
    }

    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:232 BlazeDirectories.getRelativeOutputPath
    pub fn get_relative_output_path(&self) -> String {
        Self::get_relative_output_path_of(&self.product_name)
    }

    /// Returns the output directory name, relative to the execRoot.
    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:232 BlazeDirectories.getRelativeOutputPath
    pub fn get_relative_output_path_of(product_name: &str) -> String {
        format!("{}-out", product_name)
    }

    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:261 BlazeDirectories.getProductName
    pub fn get_product_name(&self) -> &str {
        &self.product_name
    }
}
