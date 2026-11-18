//! Port of `lib/sandbox/SymlinkedSandboxedSpawn.java`.

use std::any::Any;
use std::io;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_shell::{CommandDescriptionForm, CommandFailureUtils, DescribableExecutionUnit};
use blaze_vfs::{Path, PathFragment};
use indexmap::{IndexMap, IndexSet};

use super::abstract_containerizing_sandboxed_spawn::{
    AbstractContainerizingSandboxedSpawn, ContainerizingSandboxedSpawnHooks,
};
use super::sandbox_helpers::{self, SandboxInputs, SandboxOutputs};
use super::sandbox_stash::SandboxStash;
use super::sandboxed_spawn::SandboxedSpawn;
use crate::exec::tree_deleter::TreeDeleter;

/// Creates an execRoot for a Spawn that contains input files as symlinks to their original
/// destination.
// Java: com/google/devtools/build/lib/sandbox/SymlinkedSandboxedSpawn.java:39 SymlinkedSandboxedSpawn
pub struct SymlinkedSandboxedSpawn {
    base: AbstractContainerizingSandboxedSpawn,
    /// Mnemonic of the action running in this spawn.
    mnemonic: String,
    target_label: Option<Label>,
    interactive_debug_arguments: Option<Vec<String>>,
}

impl SymlinkedSandboxedSpawn {
    // Java: com/google/devtools/build/lib/sandbox/SymlinkedSandboxedSpawn.java:48 SymlinkedSandboxedSpawn.<init>
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
        interactive_debug_arguments: Option<Vec<String>>,
        mnemonic: &str,
        target_label: Option<Label>,
    ) -> SymlinkedSandboxedSpawn {
        let base = AbstractContainerizingSandboxedSpawn::new(
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
        );
        SymlinkedSandboxedSpawn {
            base,
            mnemonic: if mnemonic.is_empty() {
                "_NoMnemonic_".to_string()
            } else {
                mnemonic.to_string()
            },
            interactive_debug_arguments,
            target_label,
        }
    }

    /// The `AbstractContainerizingSandboxedSpawn` part.
    pub fn base(&self) -> &AbstractContainerizingSandboxedSpawn {
        &self.base
    }
}

impl ContainerizingSandboxedSpawnHooks for SymlinkedSandboxedSpawn {
    // Java: com/google/devtools/build/lib/sandbox/SymlinkedSandboxedSpawn.java:80 SymlinkedSandboxedSpawn.filterInputsAndDirsToCreate
    fn filter_inputs_and_dirs_to_create(
        &self,
        inputs_to_create: &mut IndexSet<PathFragment>,
        dirs_to_create: &mut IndexSet<PathFragment>,
    ) -> io::Result<()> {
        if !SandboxStash::got_instance() {
            return Ok(());
        }
        let sandbox_contents = SandboxStash::take_stashed_sandbox(
            &self.base.sandbox_path,
            &self.mnemonic,
            self.base.get_environment(),
            &self.base.outputs,
            self.target_label.as_ref(),
        );
        self.base.sandbox_exec_root.create_directory_and_parents()?;

        let parent = self
            .base
            .sandbox_exec_root
            .get_parent_directory()
            .expect("NullPointerException: sandboxExecRoot has no parent");
        if let Some(sandbox_contents) = sandbox_contents {
            // Delete anything unnecessary, and update `inputsToCreate`/`dirsToCreate` if something
            // can be left without changes (e.g., a, symlink that already points to the right
            // destination). We're traversing from sandboxExecRoot's parent directory because
            // external repositories can now be symlinked as siblings of sandboxExecRoot when
            // --experimental_sibling_repository_layout is set.
            match sandbox_contents {
                Some(contents) => {
                    sandbox_helpers::clean_existing_with_contents(
                        &parent,
                        &self.base.inputs,
                        inputs_to_create,
                        dirs_to_create,
                        &self.base.sandbox_exec_root,
                        &*self.base.tree_deleter,
                        Some(&contents),
                    )?;
                }
                None => {
                    // No in-memory stashes enabled but there is a stash.
                    // When reusing an old sandbox, we do a full traversal of the parent directory
                    // of `sandboxExecRoot`.
                    sandbox_helpers::clean_existing(
                        &parent,
                        &self.base.inputs,
                        inputs_to_create,
                        dirs_to_create,
                        &self.base.sandbox_exec_root,
                        &*self.base.tree_deleter,
                    )?;
                    return Ok(());
                }
            }
        }

        if SandboxStash::use_in_memory_stashes() {
            SandboxStash::set_path_contents(
                &self.base.sandbox_path,
                sandbox_helpers::create_content_map(
                    &self.base.sandbox_exec_root,
                    &self.base.inputs,
                    &self.base.outputs,
                ),
            );
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/sandbox/SymlinkedSandboxedSpawn.java:128 SymlinkedSandboxedSpawn.copyFile
    fn copy_file(&self, source: &Path, target: &Path) -> io::Result<()> {
        target.create_symbolic_link(source)
    }
}

impl DescribableExecutionUnit for SymlinkedSandboxedSpawn {
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

impl SandboxedSpawn for SymlinkedSandboxedSpawn {
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

    // Java: com/google/devtools/build/lib/sandbox/SymlinkedSandboxedSpawn.java:133 SymlinkedSandboxedSpawn.delete
    fn delete(&self) {
        SandboxStash::stash_sandbox(
            &self.base.sandbox_path,
            &self.mnemonic,
            self.base.get_environment(),
            &self.base.outputs,
            &*self.base.tree_deleter,
            self.target_label.as_ref(),
        );
        self.base.delete();
    }

    // Java: com/google/devtools/build/lib/sandbox/SymlinkedSandboxedSpawn.java:141 SymlinkedSandboxedSpawn.getInteractiveDebugInstructions
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
