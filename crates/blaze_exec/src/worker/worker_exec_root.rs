//! Port of `lib/worker/WorkerExecRoot.java`.

use std::collections::BTreeSet;

use blaze_vfs::{Path, PathFragment};
use indexmap::IndexSet;

use crate::exec::tree_deleter::TreeDeleter;
use crate::sandbox::sandbox_helpers::{self, SandboxContents, SandboxInputs, SandboxOutputs};

/// Creates and manages the contents of a working directory of a persistent worker.
// Java: com/google/devtools/build/lib/worker/WorkerExecRoot.java:35 WorkerExecRoot
pub struct WorkerExecRoot {
    work_dir: Path,
    extra_dirs: Vec<PathFragment>,
    use_in_memory_tracking: bool,
    sandbox_contents: Option<SandboxContents>,
    sandbox_contents_timestamp: i64,
}

impl WorkerExecRoot {
    /// Creates a new WorkerExecRoot.
    ///
    /// `work_dir`: the directory (workspace dir) that the worker will be executing in.
    /// `extra_dirs`: directories that must survive sandbox cleanup, e.g. for things that are
    /// bind-mounted.
    // Java: com/google/devtools/build/lib/worker/WorkerExecRoot.java:50 WorkerExecRoot.<init>
    pub fn new(
        work_dir: Path,
        extra_dirs: Vec<PathFragment>,
        use_in_memory_tracking: bool,
    ) -> WorkerExecRoot {
        WorkerExecRoot {
            work_dir,
            extra_dirs,
            use_in_memory_tracking,
            sandbox_contents: None,
            sandbox_contents_timestamp: 0,
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerExecRoot.java:56 WorkerExecRoot.createFileSystem
    pub fn create_file_system(
        &mut self,
        worker_files: &BTreeSet<PathFragment>,
        inputs: &SandboxInputs,
        outputs: &SandboxOutputs,
        tree_deleter: &dyn TreeDeleter,
    ) -> std::io::Result<()> {
        self.work_dir.create_directory_and_parents()?;

        // First compute all the inputs and directories that we need. This is based only on
        // `workerFiles`, `inputs` and `outputs` and won't do any I/O.
        let mut inputs_to_create: IndexSet<PathFragment> = IndexSet::new();
        let mut dirs_to_create: IndexSet<PathFragment> = self.extra_dirs.iter().cloned().collect();
        sandbox_helpers::populate_inputs_and_dirs_to_create(
            std::iter::empty(),
            &mut inputs_to_create,
            &mut dirs_to_create,
            worker_files
                .iter()
                .chain(inputs.get_files().keys())
                .chain(inputs.get_symlinks().keys()),
            outputs,
        );

        let parent = self
            .work_dir
            .get_parent_directory()
            .expect("NullPointerException: workDir has no parent");
        // If we have information about the previous contents of the sandbox, update it to reflect
        // filesystem changes that have happened in the interim, to speed up the cleanup process
        // below.
        // TODO(tjgq): Consider doing this asynchronously in between worker invocations.
        if let Some(contents) = self.sandbox_contents.as_mut() {
            sandbox_helpers::update_content_map(&parent, self.sandbox_contents_timestamp, contents)?;
        }

        // Then do a full traversal of the parent directory of `workDir`. This will use what we
        // computed above, delete anything unnecessary and update `inputsToCreate`/`dirsToCreate`
        // if something is can be left without changes (e.g., a symlink that already points to the
        // right destination). We're traversing from workDir's parent directory because external
        // repositories can now be symlinked as siblings of workDir when
        // --experimental_sibling_repository_layout is in effect.
        sandbox_helpers::clean_existing_with_contents(
            &parent,
            inputs,
            &mut inputs_to_create,
            &mut dirs_to_create,
            &self.work_dir,
            tree_deleter,
            self.sandbox_contents.as_ref(),
        )?;

        // Finally, create anything that is still missing. This is non-strict only for historical
        // reasons, we haven't seen what would break if we make it strict.
        sandbox_helpers::create_directories(&dirs_to_create, &self.work_dir, /* strict= */ false)?;
        WorkerExecRoot::create_inputs(&inputs_to_create, inputs, &self.work_dir)?;

        // Track the sandbox contents in memory. This makes the cleanup faster in subsequent runs.
        if self.use_in_memory_tracking {
            self.sandbox_contents = Some(sandbox_helpers::create_content_map(
                &self.work_dir,
                inputs,
                outputs,
            ));
            self.sandbox_contents_timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/worker/WorkerExecRoot.java:111 WorkerExecRoot.createInputs
    pub fn create_inputs<'a>(
        inputs_to_create: impl IntoIterator<Item = &'a PathFragment>,
        inputs: &SandboxInputs,
        dir: &Path,
    ) -> std::io::Result<()> {
        for fragment in inputs_to_create {
            if blaze_shell::thread_interrupt::interrupted() {
                return Err(sandbox_helpers::interrupted_error(None));
            }
            let key = dir.get_relative(fragment);
            if let Some(file_dest) = inputs.get_files().get(fragment) {
                match file_dest {
                    Some(file_dest) => key.create_symbolic_link(file_dest)?,
                    None => crate::analysis::actions::file_system_utils::create_empty_file(&key)?,
                }
            } else if let Some(symlink_dest) = inputs.get_symlinks().get(fragment) {
                key.create_symbolic_link_to_fragment(symlink_dest)?;
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/worker/WorkerExecRoot.java:131 WorkerExecRoot.copyOutputs
    pub fn copy_outputs(&self, exec_root: &Path, outputs: &SandboxOutputs) -> std::io::Result<()> {
        sandbox_helpers::move_outputs(outputs, &self.work_dir, exec_root)
    }
}
