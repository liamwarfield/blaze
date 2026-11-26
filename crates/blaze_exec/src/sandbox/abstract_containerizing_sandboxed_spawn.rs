//! Port of `lib/sandbox/AbstractContainerizingSandboxedSpawn.java`.

use std::io;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use blaze_util::profiler::{Profiler, ProfilerTask};
use blaze_vfs::{Path, PathFragment};
use indexmap::{IndexMap, IndexSet};

use super::fs_utils;
use super::sandbox_helpers::{self, SandboxInputs, SandboxOutputs, check_interrupted};
use super::sandbox_stash::SandboxStash;
use crate::exec::tree_deleter::TreeDeleter;

/// The abstract methods of `AbstractContainerizingSandboxedSpawn` that subclasses provide.
// Java: com/google/devtools/build/lib/sandbox/AbstractContainerizingSandboxedSpawn.java:39 AbstractContainerizingSandboxedSpawn
pub trait ContainerizingSandboxedSpawnHooks {
    /// `filterInputsAndDirsToCreate(inputsToCreate, dirsToCreate)`: allows subclasses to filter
    /// out inputs and dirs that don't need to be created.
    // Java: com/google/devtools/build/lib/sandbox/AbstractContainerizingSandboxedSpawn.java:146 AbstractContainerizingSandboxedSpawn.filterInputsAndDirsToCreate
    fn filter_inputs_and_dirs_to_create(
        &self,
        _inputs_to_create: &mut IndexSet<PathFragment>,
        _dirs_to_create: &mut IndexSet<PathFragment>,
    ) -> io::Result<()> {
        Ok(())
    }

    /// `copyFile(source, target)`.
    fn copy_file(&self, source: &Path, target: &Path) -> io::Result<()>;
}

/// Implements the general flow of a sandboxed spawn that uses a container directory to build an
/// execution root for a spawn.
///
/// The subclass-specific parts are a [`ContainerizingSandboxedSpawnHooks`] passed to
/// [`Self::create_file_system`].
// Java: com/google/devtools/build/lib/sandbox/AbstractContainerizingSandboxedSpawn.java:39 AbstractContainerizingSandboxedSpawn
pub struct AbstractContainerizingSandboxedSpawn {
    pub sandbox_path: Path,
    pub sandbox_exec_root: Path,
    arguments: Vec<String>,
    environment: IndexMap<String, String>,
    pub inputs: SandboxInputs,
    pub outputs: SandboxOutputs,
    writable_dirs: IndexSet<Path>,
    pub tree_deleter: Arc<dyn TreeDeleter>,
    sandbox_debug_path: Option<Path>,
    statistics_path: Option<Path>,
    mnemonic: String,
}

impl AbstractContainerizingSandboxedSpawn {
    // Java: com/google/devtools/build/lib/sandbox/AbstractContainerizingSandboxedSpawn.java:53 AbstractContainerizingSandboxedSpawn.<init>
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
        mnemonic: String,
    ) -> AbstractContainerizingSandboxedSpawn {
        AbstractContainerizingSandboxedSpawn {
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
            mnemonic,
        }
    }

    /// `getSandboxExecRoot()`.
    pub fn get_sandbox_exec_root(&self) -> &Path {
        &self.sandbox_exec_root
    }

    /// `getArguments()`.
    pub fn get_arguments(&self) -> &[String] {
        &self.arguments
    }

    /// `getEnvironment()`.
    pub fn get_environment(&self) -> &IndexMap<String, String> {
        &self.environment
    }

    /// `getSandboxDebugPath()`.
    pub fn get_sandbox_debug_path(&self) -> Option<&Path> {
        self.sandbox_debug_path.as_ref()
    }

    /// `getStatisticsPath()`.
    pub fn get_statistics_path(&self) -> Option<&Path> {
        self.statistics_path.as_ref()
    }

    /// `getMnemonic()`.
    pub fn get_mnemonic(&self) -> &str {
        &self.mnemonic
    }

    /// `createFileSystem()`.
    // Java: com/google/devtools/build/lib/sandbox/AbstractContainerizingSandboxedSpawn.java:111 AbstractContainerizingSandboxedSpawn.createFileSystem
    pub fn create_file_system(
        &self,
        hooks: &dyn ContainerizingSandboxedSpawnHooks,
    ) -> io::Result<()> {
        // First compute all the inputs and directories that we need. This is based only on
        // `workerFiles`, `inputs` and `outputs` and won't do any I/O.
        let mut inputs_to_create: IndexSet<PathFragment> = IndexSet::new();
        let mut dirs_to_create: IndexSet<PathFragment> = IndexSet::new();
        let writable_sandbox_dirs: IndexSet<PathFragment> = self
            .writable_dirs
            .iter()
            .filter(|p| p.starts_with(&self.sandbox_exec_root))
            .map(|p| p.relative_to(&self.sandbox_exec_root))
            .collect();
        let input_files: Vec<PathFragment> = self
            .inputs
            .get_files()
            .keys()
            .chain(self.inputs.get_symlinks().keys())
            .cloned()
            .collect();
        // Java: com/google/devtools/build/lib/sandbox/AbstractContainerizingSandboxedSpawn.java:121 AbstractContainerizingSandboxedSpawn.createFileSystem
        {
            let _c = Profiler::instance()
                .profile(ProfilerTask::INFO, "sandbox.populateInputsAndDirsToCreate");
            sandbox_helpers::populate_inputs_and_dirs_to_create(
                &writable_sandbox_dirs,
                &mut inputs_to_create,
                &mut dirs_to_create,
                &input_files,
                &self.outputs,
            );
        }

        // Java: com/google/devtools/build/lib/sandbox/AbstractContainerizingSandboxedSpawn.java:131 AbstractContainerizingSandboxedSpawn.createFileSystem
        {
            let _c = Profiler::instance()
                .profile(ProfilerTask::INFO, "sandbox.filterInputsAndDirsToCreate");
            // Allow subclasses to filter out inputs and dirs that don't need to be created.
            hooks.filter_inputs_and_dirs_to_create(&mut inputs_to_create, &mut dirs_to_create)?;
        }

        // Finally create what needs creating.
        // Java: com/google/devtools/build/lib/sandbox/AbstractContainerizingSandboxedSpawn.java:137 AbstractContainerizingSandboxedSpawn.createFileSystem
        {
            let _c = Profiler::instance().profile(ProfilerTask::INFO, "sandbox.createDirectories");
            sandbox_helpers::create_directories(
                &dirs_to_create,
                &self.sandbox_exec_root,
                /* strict= */ true,
            )?;
        }
        // Java: com/google/devtools/build/lib/sandbox/AbstractContainerizingSandboxedSpawn.java:140 AbstractContainerizingSandboxedSpawn.createFileSystem
        {
            let _c = Profiler::instance().profile(ProfilerTask::INFO, "sandbox.createInputs");
            self.create_inputs(hooks, &inputs_to_create, &self.inputs)?;
        }
        let now_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        SandboxStash::set_last_modified(&self.sandbox_path, now_millis);
        Ok(())
    }

    /// Creates all inputs needed for this spawn's sandbox.
    ///
    /// `inputs_to_create`: the inputs that actually need to be created. Some inputs may already
    /// exist if we're reusing a previously existing sandbox. `inputs`: all the inputs for this
    /// spawn.
    // Java: com/google/devtools/build/lib/sandbox/AbstractContainerizingSandboxedSpawn.java:157 AbstractContainerizingSandboxedSpawn.createInputs
    pub fn create_inputs<'a>(
        &self,
        hooks: &dyn ContainerizingSandboxedSpawnHooks,
        inputs_to_create: impl IntoIterator<Item = &'a PathFragment>,
        inputs: &SandboxInputs,
    ) -> io::Result<()> {
        for fragment in inputs_to_create {
            check_interrupted(Some("Interrupted creating inputs"))?;
            let key = self.sandbox_exec_root.get_relative(fragment);
            if let Some(file_dest) = inputs.get_files().get(fragment) {
                match file_dest {
                    Some(file_dest) => hooks.copy_file(file_dest, &key)?,
                    None => fs_utils::create_empty_file(&key)?,
                }
            } else if let Some(symlink_dest) = inputs.get_symlinks().get(fragment) {
                key.create_symbolic_link_to_fragment(symlink_dest)?;
            }
        }
        Ok(())
    }

    /// `copyOutputs(execRoot)`.
    // Java: com/google/devtools/build/lib/sandbox/AbstractContainerizingSandboxedSpawn.java:183 AbstractContainerizingSandboxedSpawn.copyOutputs
    pub fn copy_outputs(&self, exec_root: &Path) -> io::Result<()> {
        sandbox_helpers::move_outputs(&self.outputs, &self.sandbox_exec_root, exec_root)
    }

    /// `delete()`.
    // Java: com/google/devtools/build/lib/sandbox/AbstractContainerizingSandboxedSpawn.java:188 AbstractContainerizingSandboxedSpawn.delete
    pub fn delete(&self) {
        // On error:
        // This usually means that the Spawn itself exited, but still has children running that
        // we couldn't wait for, which now block deletion of the sandbox directory. On Linux
        // this should never happen, as we use PID namespaces and where they are not available
        // the subreaper feature to make sure all children have been reliably killed before
        // returning, but on other OS this might not always work. The SandboxModule will try to
        // delete them again when the build is all done, at which point it hopefully works, so
        // let's just go on here.
        let _ = self.tree_deleter.delete_tree(&self.sandbox_path);
    }
}
