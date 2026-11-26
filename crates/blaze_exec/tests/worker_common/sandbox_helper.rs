//! Port of src/test/java/com/google/devtools/build/lib/worker/SandboxHelper.java.
//!
//! PORT: `addAndCreateVirtualInput` is not ported (no ported test uses it).
#![allow(dead_code)]

use std::collections::BTreeSet;

use blaze_exec::sandbox::sandbox_helpers::{SandboxInputs, SandboxOutputs};
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;

/// Helper to create sandbox inputs, outputs and worker files for the worker tests.
// Java: src/test/java/com/google/devtools/build/lib/worker/SandboxHelper.java:39 SandboxHelper
pub struct SandboxHelper {
    /// Map from workdir-relative input path to optional real file path.
    inputs: IndexMap<PathFragment, Option<Path>>,
    symlinks: IndexMap<PathFragment, PathFragment>,
    worker_files: IndexMap<PathFragment, Path>,
    output_files: Vec<PathFragment>,
    output_dirs: Vec<PathFragment>,
    /// The global execRoot.
    pub exec_root: Path,
    /// The worker process's sandbox root.
    pub work_dir: Path,
}

/// `FileSystemUtils.writeContentAsLatin1` (the contents are ASCII).
pub fn write_content_as_latin1(path: &Path, contents: &str) {
    std::fs::write(path.get_path_string(), contents.as_bytes()).unwrap();
}

/// `FileSystemUtils.writeIsoLatin1(path, lines...)`: every line ends with `\n`, and the parent
/// directories are created.
pub fn write_iso_latin1(path: &Path, lines: &[&str]) {
    path.get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    let mut content = String::new();
    for line in lines {
        content.push_str(line);
        content.push('\n');
    }
    write_content_as_latin1(path, &content);
}

/// `FileSystemUtils.readContent(path, charset)`.
pub fn read_content(path: &Path) -> String {
    std::fs::read_to_string(path.get_path_string()).unwrap()
}

/// `FileSystemUtils.createEmptyFile`.
pub fn create_empty_file(path: &Path) {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path.get_path_string())
        .unwrap();
}

impl SandboxHelper {
    // Java: src/test/java/com/google/devtools/build/lib/worker/SandboxHelper.java:58 SandboxHelper.<init>
    pub fn new(exec_root: &Path, work_dir: &Path) -> SandboxHelper {
        SandboxHelper {
            inputs: IndexMap::new(),
            symlinks: IndexMap::new(),
            worker_files: IndexMap::new(),
            output_files: Vec::new(),
            output_dirs: Vec::new(),
            exec_root: exec_root.clone(),
            work_dir: work_dir.clone(),
        }
    }

    /// Adds a regular input file at `relative_path` under `work_dir`, with the real file at
    /// `workspace_path` under `exec_root`.
    // Java: src/test/java/com/google/devtools/build/lib/worker/SandboxHelper.java:67 SandboxHelper.addInputFile
    pub fn add_input_file(mut self, relative_path: &str, workspace_path: Option<&str>) -> Self {
        self.inputs.insert(
            PathFragment::create(relative_path),
            workspace_path.map(|w| self.exec_root.get_relative_str(w)),
        );
        self
    }

    /// Adds a regular input file and creates the real file under `exec_root` with `contents`.
    // Java: src/test/java/com/google/devtools/build/lib/worker/SandboxHelper.java:80 SandboxHelper.addAndCreateInputFile
    pub fn add_and_create_input_file(
        self,
        relative_path: &str,
        workspace_path: &str,
        contents: &str,
    ) -> Self {
        let this = self.add_input_file(relative_path, Some(workspace_path));
        let abs_path = this.exec_root.get_relative_str(workspace_path);
        abs_path
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        write_content_as_latin1(&abs_path, contents);
        this
    }

    /// Adds a symlink to the inputs.
    // Java: src/test/java/com/google/devtools/build/lib/worker/SandboxHelper.java:107 SandboxHelper.addSymlink
    pub fn add_symlink(mut self, relative_path: &str, link_to: &str) -> Self {
        self.symlinks.insert(
            PathFragment::create(relative_path),
            PathFragment::create(link_to),
        );
        self
    }

    /// Adds an output file without creating it.
    // Java: src/test/java/com/google/devtools/build/lib/worker/SandboxHelper.java:114 SandboxHelper.addOutput
    pub fn add_output(mut self, relative_path: &str) -> Self {
        self.output_files.push(PathFragment::create(relative_path));
        self
    }

    /// Adds an output directory without creating it.
    // Java: src/test/java/com/google/devtools/build/lib/worker/SandboxHelper.java:121 SandboxHelper.addOutputDir
    pub fn add_output_dir(mut self, relative_path: &str) -> Self {
        let p = if relative_path.ends_with('/') {
            relative_path.to_string()
        } else {
            format!("{relative_path}/")
        };
        self.output_dirs.push(PathFragment::create(&p));
        self
    }

    /// Adds a worker file that is created under `exec_root` and referenced under the `work_dir`.
    // Java: src/test/java/com/google/devtools/build/lib/worker/SandboxHelper.java:132 SandboxHelper.addWorkerFile
    pub fn add_worker_file(mut self, relative_path: &str) -> Self {
        let abs_path = self.exec_root.get_relative_str(relative_path);
        self.worker_files
            .insert(PathFragment::create(relative_path), abs_path);
        self
    }

    /// Adds a worker file and writes `contents` to it under `exec_root`.
    // Java: src/test/java/com/google/devtools/build/lib/worker/SandboxHelper.java:143 SandboxHelper.addAndCreateWorkerFile
    pub fn add_and_create_worker_file(self, relative_path: &str, contents: &str) -> Self {
        let this = self.add_worker_file(relative_path);
        let abs_path = this.exec_root.get_relative_str(relative_path);
        abs_path
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        write_content_as_latin1(&abs_path, contents);
        this
    }

    /// Creates a file with `contents` at `relative_path` under the `work_dir`.
    // Java: src/test/java/com/google/devtools/build/lib/worker/SandboxHelper.java:157 SandboxHelper.createExecRootFile
    pub fn create_exec_root_file(self, relative_path: &str, contents: &str) -> Self {
        let abs_path = self.work_dir.get_relative_str(relative_path);
        abs_path
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        write_content_as_latin1(&abs_path, contents);
        self
    }

    /// Creates a file with `contents` at `workspace_dir_path` under the `exec_root`.
    // Java: src/test/java/com/google/devtools/build/lib/worker/SandboxHelper.java:169 SandboxHelper.createWorkspaceDirFile
    pub fn create_workspace_dir_file(self, workspace_dir_path: &str, contents: &str) -> Self {
        let abs_path = self.exec_root.get_relative_str(workspace_dir_path);
        abs_path
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        write_content_as_latin1(&abs_path, contents);
        self
    }

    /// Creates a symlink from within the `work_dir`. The destination is just what's written into
    /// the symlink and thus relative to the created symlink.
    // Java: src/test/java/com/google/devtools/build/lib/worker/SandboxHelper.java:182 SandboxHelper.createSymlink
    pub fn create_symlink(self, relative_path: &str, relative_destination: &str) -> Self {
        let from_path = self.work_dir.get_relative_str(relative_path);
        // FileSystemUtils.ensureSymbolicLink: creates the parent directories first.
        from_path
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        from_path
            .create_symbolic_link_to_fragment(&PathFragment::create(relative_destination))
            .unwrap();
        self
    }

    // Java: src/test/java/com/google/devtools/build/lib/worker/SandboxHelper.java:188 SandboxHelper.getSandboxInputs
    pub fn get_sandbox_inputs(&self) -> SandboxInputs {
        SandboxInputs::new(self.inputs.clone(), IndexMap::new(), self.symlinks.clone())
    }

    // Java: src/test/java/com/google/devtools/build/lib/worker/SandboxHelper.java:192 SandboxHelper.getSandboxOutputs
    pub fn get_sandbox_outputs(&self) -> SandboxOutputs {
        SandboxOutputs::create_from_sets(self.output_files.clone(), self.output_dirs.clone())
    }

    // Java: src/test/java/com/google/devtools/build/lib/worker/SandboxHelper.java:197 SandboxHelper.getWorkerFiles
    pub fn get_worker_files(&self) -> BTreeSet<PathFragment> {
        self.worker_files.keys().cloned().collect()
    }
}
