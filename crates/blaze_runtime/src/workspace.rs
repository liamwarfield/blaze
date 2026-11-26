//! `BlazeWorkspace`: the workspace-specific state of the server (directories, caches).
//!
//! PORT: W1 subset. The action cache, bin tools, syscall cache and package locator are added
//! with the waves that need them; of the Skyframe executor, what outlives a command is
//! `blaze_sky_top::incremental::SkyframeIncrementalState`.

use std::io::Write;

use blaze_vfs::Path;

use crate::directories::BlazeDirectories;

/// Builder class to create a `BlazeWorkspace`: what the modules add in `workspaceInit`.
///
/// PORT: of Java's builder only the diff awareness factories (`addDiffAwarenessFactory`); the
/// rest builds the server's Skyframe executor, which Blaze creates per command.
// Java: com/google/devtools/build/lib/runtime/WorkspaceBuilder.java:48 WorkspaceBuilder
#[derive(Default)]
pub struct WorkspaceBuilder {
    diff_awareness_factories:
        Vec<std::sync::Arc<dyn blaze_sky_loading::diff_awareness::DiffAwarenessFactory>>,
}

impl WorkspaceBuilder {
    /// Add a `DiffAwareness` factory. These will be used to determine which files, if any,
    /// changed between Blaze commands. Note that these factories are attempted in the order in
    /// which they are added to this class, so order matters - in order to guarantee a specific
    /// order, only a single module should add such factories.
    // Java: com/google/devtools/build/lib/runtime/WorkspaceBuilder.java:202 WorkspaceBuilder.addDiffAwarenessFactory
    pub fn add_diff_awareness_factory(
        &mut self,
        factory: std::sync::Arc<dyn blaze_sky_loading::diff_awareness::DiffAwarenessFactory>,
    ) -> &mut Self {
        self.diff_awareness_factories.push(factory);
        self
    }
}

/// This class represents a workspace, and contains operations and data related to it.
// Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:68 BlazeWorkspace
pub struct BlazeWorkspace {
    directories: BlazeDirectories,
    /// Java `skyframeExecutor`: what of it outlives a command (the kept graph and its sync
    /// state, see `blaze_sky_top::incremental`).
    incremental_state: std::sync::Arc<blaze_sky_top::incremental::SkyframeIncrementalState>,
}

impl BlazeWorkspace {
    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:69 BlazeWorkspace.DO_NOT_BUILD_FILE_NAME
    pub const DO_NOT_BUILD_FILE_NAME: &'static str = "DO_NOT_BUILD_HERE";

    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:156 BlazeWorkspace.<init>
    pub fn new(directories: BlazeDirectories, builder: WorkspaceBuilder) -> Self {
        let ws = BlazeWorkspace {
            directories,
            // PORT: Blaze-only (decision D6): `BLAZE_INCREMENTAL` of the server's environment.
            incremental_state: std::sync::Arc::new(
                blaze_sky_top::incremental::SkyframeIncrementalState::new(
                    blaze_sky_top::incremental::KeepGraphMode::from_process_environment(),
                    builder.diff_awareness_factories,
                ),
            ),
        };
        if ws.directories.in_workspace() {
            ws.write_output_base_readme_file();
            ws.write_do_not_build_here_files();
        }
        // PORT: outputBaseFilesystemTypeName (FileSystemUtils.getFileSystem) is only used for
        // logging and BEP; not ported.
        ws
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:200 BlazeWorkspace.getDirectories
    pub fn get_directories(&self) -> &BlazeDirectories {
        &self.directories
    }

    /// Returns the working directory of the server.
    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:222 BlazeWorkspace.getWorkspace
    pub fn get_workspace(&self) -> Option<&Path> {
        self.directories.get_working_directory()
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:231 BlazeWorkspace.getOutputBase
    pub fn get_output_base(&self) -> &Path {
        self.directories.get_output_base()
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:244 BlazeWorkspace.getInstallBase
    pub fn get_install_base(&self) -> &Path {
        self.directories.get_install_base()
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:254 BlazeWorkspace.getActionCacheDirectory
    fn get_action_cache_directory(&self) -> Path {
        self.get_output_base().get_child("action_cache")
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:264 BlazeWorkspace.getCorruptedActionCacheDirectory
    fn get_corrupted_action_cache_directory(&self) -> Path {
        self.get_output_base().get_child("action_cache.bad")
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:274 BlazeWorkspace.getActionCacheTmpDirectory
    fn get_action_cache_tmp_directory(&self) -> Path {
        self.get_output_base().get_child("action_cache.tmp")
    }

    /// The Skyframe state that outlives a command.
    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:205 BlazeWorkspace.getSkyframeExecutor
    pub fn get_incremental_state(
        &self,
    ) -> &std::sync::Arc<blaze_sky_top::incremental::SkyframeIncrementalState> {
        &self.incremental_state
    }

    /// Reinitializes the Skyframe evaluator, dropping all previously computed values.
    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:366 BlazeWorkspace.resetEvaluator
    pub fn reset_evaluator(&self) {
        self.incremental_state.reset_evaluator();
    }

    /// Removes in-memory and on-disk action caches.
    /// PORT: the in-memory action cache does not exist yet; the on-disk directories are deleted
    /// as in Java.
    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:371 BlazeWorkspace.clearCaches
    pub fn clear_caches(&self) -> std::io::Result<()> {
        // actionCache = null
        crate::idle_tasks::note_persistent_action_cache_dropped();
        self.get_action_cache_directory().delete_tree()?;
        self.get_corrupted_action_cache_directory().delete_tree()?;
        self.get_action_cache_tmp_directory().delete_tree()?;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:184 BlazeWorkspace.writeOutputBaseReadmeFile
    fn write_output_base_readme_file(&self) {
        let workspace = self.get_workspace().expect("checkNotNull");
        let output_base_readme_file = self.get_output_base().get_relative_str("README");
        let lines = [
            format!("WORKSPACE: {}", workspace.get_path_string()),
            String::new(),
            "The first line of this file is intentionally easy to parse for various".to_string(),
            "interactive scripting and debugging purposes.  But please DO NOT write programs"
                .to_string(),
            "that exploit it, as they will be broken by design: it is not possible to".to_string(),
            "reverse engineer the set of source trees or the --package_path from the output"
                .to_string(),
            "tree, and if you attempt it, you will fail, creating subtle and".to_string(),
            "hard-to-diagnose bugs, that will no doubt get blamed on changes made by the"
                .to_string(),
            "Bazel team.".to_string(),
            String::new(),
            "This directory was generated by Bazel.".to_string(),
            "Do not attempt to modify or delete any files in this directory.".to_string(),
            "Among other issues, Bazel's file system caching assumes that".to_string(),
            "only Bazel will modify this directory and the files in it,".to_string(),
            "so if you change anything here you may mess up Bazel's cache.".to_string(),
        ];
        // FileSystemUtils.writeIsoLatin1 writes each line followed by '\n'.
        let mut content = String::new();
        for line in &lines {
            content.push_str(line);
            content.push('\n');
        }
        // Failures are only logged (logger.atWarning) in Java.
        let _ = output_base_readme_file
            .get_output_stream()
            .and_then(|mut out| {
                out.write_all(&blaze_util::string_encoding::latin1_encode_lossy(&content))
            });
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:185 BlazeWorkspace.writeDoNotBuildHereFile
    fn write_do_not_build_here_file(&self, file_path: &Path) {
        let workspace = self.get_workspace().expect("checkNotNull");
        if let Some(parent) = file_path.get_parent_directory()
            && parent.create_directory_and_parents().is_err()
        {
            return;
        }
        // Failures are only logged (logger.atWarning) in Java.
        // FileSystemUtils.writeContent(filePath, ISO_8859_1, getWorkspace().toString()).
        let _ = file_path.get_output_stream().and_then(|mut out| {
            out.write_all(&blaze_util::string_encoding::latin1_encode_lossy(
                workspace.get_path_string(),
            ))
        });
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:185 BlazeWorkspace.writeDoNotBuildHereFile
    fn write_do_not_build_here_files(&self) {
        self.write_do_not_build_here_file(
            &self
                .get_output_base()
                .get_relative_str(Self::DO_NOT_BUILD_FILE_NAME),
        );
        self.write_do_not_build_here_file(
            &self
                .get_output_base()
                .get_relative_str("execroot")
                .get_relative_str(Self::DO_NOT_BUILD_FILE_NAME),
        );
    }
}
