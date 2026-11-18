//! Port of `lib/sandbox/HardlinkedSandboxedSpawn.java`.

use std::any::Any;
use std::io;
use std::sync::Arc;

use blaze_shell::{CommandDescriptionForm, CommandFailureUtils, DescribableExecutionUnit};
use blaze_vfs::{Path, Symlinks};
use indexmap::{IndexMap, IndexSet};

use super::abstract_containerizing_sandboxed_spawn::{
    AbstractContainerizingSandboxedSpawn, ContainerizingSandboxedSpawnHooks,
};
use super::fs_utils;
use super::sandbox_helpers::{SandboxInputs, SandboxOutputs};
use super::sandboxed_spawn::SandboxedSpawn;
use crate::exec::tree_deleter::TreeDeleter;

/// Creates an execRoot for a Spawn that contains input files as hardlinks to their original
/// destination.
// Java: com/google/devtools/build/lib/sandbox/HardlinkedSandboxedSpawn.java:39 HardlinkedSandboxedSpawn
pub struct HardlinkedSandboxedSpawn {
    base: AbstractContainerizingSandboxedSpawn,
    sandbox_debug: bool,
    interactive_debug_arguments: Option<Vec<String>>,
}

impl HardlinkedSandboxedSpawn {
    // Java: com/google/devtools/build/lib/sandbox/HardlinkedSandboxedSpawn.java:44 HardlinkedSandboxedSpawn.<init>
    pub fn new(
        sandbox_path: Path,
        sandbox_exec_root: Path,
        arguments: Vec<String>,
        environment: IndexMap<String, String>,
        inputs: SandboxInputs,
        outputs: SandboxOutputs,
        writable_dirs: IndexSet<Path>,
        tree_deleter: Arc<dyn TreeDeleter>,
        sandbox_debug_path: Option<Path>,
        statistics_path: Option<Path>,
        sandbox_debug: bool,
        interactive_debug_arguments: Option<Vec<String>>,
        mnemonic: &str,
    ) -> HardlinkedSandboxedSpawn {
        HardlinkedSandboxedSpawn {
            base: AbstractContainerizingSandboxedSpawn::new(
                sandbox_path,
                sandbox_exec_root,
                arguments,
                environment,
                inputs,
                outputs,
                writable_dirs,
                tree_deleter,
                sandbox_debug_path,
                statistics_path,
                mnemonic.to_string(),
            ),
            sandbox_debug,
            interactive_debug_arguments,
        }
    }

    /// The `AbstractContainerizingSandboxedSpawn` part.
    pub fn base(&self) -> &AbstractContainerizingSandboxedSpawn {
        &self.base
    }

    /// Recursively creates hardlinks for all files in `source` path, in `target` path. Symlinks
    /// are resolved. If files is located on another disk, hardlink will fail and a copy will be
    /// made instead. Panics (Java `IllegalArgumentException`) if source path is a subdirectory of
    /// target path.
    // Java: com/google/devtools/build/lib/sandbox/HardlinkedSandboxedSpawn.java:85 HardlinkedSandboxedSpawn.hardLinkRecursive
    fn hard_link_recursive(&self, source: &Path, target: &Path) -> io::Result<()> {
        let mut source = source.clone();
        let mut stat = source.stat_with(Symlinks::Nofollow)?;

        if stat.is_symbolic_link() {
            source = source.resolve_symbolic_links()?;
            stat = source.stat()?;
        }

        if stat.is_file() {
            if source.create_hard_link(target).is_err() {
                // PORT: skipped, tracing (Java log) (when `sandbox_debug`: "File %s could not be hardlinked,
                // file will be copied instead.").
                let _ = self.sandbox_debug;
                fs_utils::copy_file(&source, target)?;
            }
        } else if stat.is_directory() {
            if source.starts_with(target) {
                panic!(
                    "IllegalArgumentException: {} is a subdirectory of {}",
                    source, target
                );
            }
            target.create_directory()?;
            let entries = source.get_directory_entries()?;
            for entry in entries {
                let to_path = target.get_child(entry.get_base_name());
                self.hard_link_recursive(&entry, &to_path)?;
            }
        }
        Ok(())
    }
}

impl ContainerizingSandboxedSpawnHooks for HardlinkedSandboxedSpawn {
    // Java: com/google/devtools/build/lib/sandbox/HardlinkedSandboxedSpawn.java:75 HardlinkedSandboxedSpawn.copyFile
    fn copy_file(&self, source: &Path, target: &Path) -> io::Result<()> {
        self.hard_link_recursive(source, target)
    }
}

impl DescribableExecutionUnit for HardlinkedSandboxedSpawn {
    fn get_arguments(&self) -> Vec<String> {
        self.base.get_arguments().to_vec()
    }
    fn get_environment(&self) -> Option<IndexMap<String, String>> {
        Some(self.base.get_environment().clone())
    }
    fn get_mnemonic(&self) -> String {
        self.base.get_mnemonic().to_string()
    }
}

impl SandboxedSpawn for HardlinkedSandboxedSpawn {
    fn get_sandbox_exec_root(&self) -> &Path {
        self.base.get_sandbox_exec_root()
    }
    fn get_sandbox_debug_path(&self) -> Option<&Path> {
        self.base.get_sandbox_debug_path()
    }
    fn get_statistics_path(&self) -> Option<&Path> {
        self.base.get_statistics_path()
    }
    fn create_file_system(&self) -> io::Result<()> {
        self.base.create_file_system(self)
    }
    fn copy_outputs(&self, exec_root: &Path) -> io::Result<()> {
        self.base.copy_outputs(exec_root)
    }
    fn delete(&self) {
        self.base.delete();
    }

    // Java: com/google/devtools/build/lib/sandbox/HardlinkedSandboxedSpawn.java:117 HardlinkedSandboxedSpawn.getInteractiveDebugInstructions
    fn get_interactive_debug_instructions(&self) -> Option<String> {
        let interactive_debug_arguments = self.interactive_debug_arguments.as_ref()?;
        Some(format!(
            "Run this command to start an interactive shell in an identical sandboxed environment:\n{}",
            CommandFailureUtils::describe_command(
                CommandDescriptionForm::Complete,
                /* pretty_print_args= */ false,
                interactive_debug_arguments.as_slice(),
                Some(self.base.get_environment()),
                /* environment_variables_to_clear= */ None,
                /* cwd= */ Some(self.base.sandbox_exec_root.get_path_string()),
                /* configuration_checksum= */ None,
                /* execution_platform_label= */ None,
                /* spawn_runner= */ None,
            )
        ))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
