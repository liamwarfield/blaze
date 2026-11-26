//! Port of `lib/sandbox/CopyingSandboxedSpawn.java`.

use std::any::Any;
use std::io;
use std::sync::Arc;

use blaze_shell::DescribableExecutionUnit;
use blaze_vfs::{Path, Symlinks};
use indexmap::{IndexMap, IndexSet};

use super::abstract_containerizing_sandboxed_spawn::{
    AbstractContainerizingSandboxedSpawn, ContainerizingSandboxedSpawnHooks,
};
use super::fs_utils;
use super::sandbox_helpers::{SandboxInputs, SandboxOutputs};
use super::sandboxed_spawn::SandboxedSpawn;
use crate::exec::tree_deleter::TreeDeleter;

/// Creates an execRoot for a Spawn that contains input files as copies from their original
/// source.
// Java: com/google/devtools/build/lib/sandbox/CopyingSandboxedSpawn.java:33 CopyingSandboxedSpawn
pub struct CopyingSandboxedSpawn {
    base: AbstractContainerizingSandboxedSpawn,
    success_callback: Box<dyn Fn() + Send + Sync>,
}

impl CopyingSandboxedSpawn {
    // Java: com/google/devtools/build/lib/sandbox/CopyingSandboxedSpawn.java:36 CopyingSandboxedSpawn.<init>
    #[allow(clippy::too_many_arguments)]
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
        success_callback: Box<dyn Fn() + Send + Sync>,
        mnemonic: &str,
    ) -> CopyingSandboxedSpawn {
        CopyingSandboxedSpawn {
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
            success_callback,
        }
    }
}

impl ContainerizingSandboxedSpawnHooks for CopyingSandboxedSpawn {
    // Java: com/google/devtools/build/lib/sandbox/CopyingSandboxedSpawn.java:70 CopyingSandboxedSpawn.copyFile
    fn copy_file(&self, source: &Path, target: &Path) -> io::Result<()> {
        let stat = source.stat_with(Symlinks::Nofollow)?;
        if stat.is_symbolic_link() || stat.is_file() {
            fs_utils::copy_file(source, target)?;
        } else if stat.is_directory() {
            target.create_directory()?;
            fs_utils::copy_trees_below(source, target)?;
        }
        Ok(())
    }
}

impl DescribableExecutionUnit for CopyingSandboxedSpawn {
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

impl SandboxedSpawn for CopyingSandboxedSpawn {
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
    // Java: com/google/devtools/build/lib/sandbox/CopyingSandboxedSpawn.java:64 CopyingSandboxedSpawn.copyOutputs
    fn copy_outputs(&self, exec_root: &Path) -> io::Result<()> {
        (self.success_callback)();
        self.base.copy_outputs(exec_root)
    }
    fn delete(&self) {
        self.base.delete();
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
