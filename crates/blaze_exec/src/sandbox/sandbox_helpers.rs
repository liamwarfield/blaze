//! Port of `lib/sandbox/SandboxHelpers.java` (the parts used on Linux).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};

use blaze_cmdline::label_constants::EXPERIMENTAL_EXTERNAL_PATH_PREFIX;
use blaze_protos::failure_details::{FailureDetail, failure_detail, sandbox};
use blaze_shell::thread_interrupt;
use blaze_vfs::{DirentType, FileSystemExt, Path, PathFragment, Symlinks};
use indexmap::{IndexMap, IndexSet};

use crate::actions::{ActionInputRef, InputMapping, Spawn, UserExecException};
use crate::exec::tree_deleter::TreeDeleter;

/// `SandboxHelpers.INACCESSIBLE_HELPER_DIR`.
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:86 SandboxHelpers.INACCESSIBLE_HELPER_DIR
pub const INACCESSIBLE_HELPER_DIR: &str = "inaccessibleHelperDir";
/// `SandboxHelpers.INACCESSIBLE_HELPER_FILE`.
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:87 SandboxHelpers.INACCESSIBLE_HELPER_FILE
pub const INACCESSIBLE_HELPER_FILE: &str = "inaccessibleHelperFile";

// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:91 SandboxHelpers.warnedAboutMovesBeingCopies
static WARNED_ABOUT_MOVES_BEING_COPIES: AtomicBool = AtomicBool::new(false);

/// The payload of the `io::Error` that stands for Java's `InterruptedException` in the sandbox
/// code, whose methods throw both `IOException` and `InterruptedException`.
#[derive(Debug)]
pub struct SandboxInterrupted(pub Option<String>);

impl fmt::Display for SandboxInterrupted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Some(m) => write!(f, "{m}"),
            None => f.write_str("interrupted"),
        }
    }
}

impl std::error::Error for SandboxInterrupted {}

/// `new InterruptedException(message)`, carried through an `io::Result`.
pub fn interrupted_error(message: Option<&str>) -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        SandboxInterrupted(message.map(str::to_string)),
    )
}

/// Whether `e` was made by [`interrupted_error`] (Java `catch (InterruptedException e)`).
pub fn is_interrupted_error(e: &io::Error) -> bool {
    e.get_ref()
        .is_some_and(|inner| inner.is::<SandboxInterrupted>())
}

/// `if (Thread.interrupted()) throw new InterruptedException();`
pub(crate) fn check_interrupted(message: Option<&str>) -> io::Result<()> {
    if thread_interrupt::interrupted() {
        return Err(interrupted_error(message));
    }
    Ok(())
}

/// `DirectoryCopier`: copies a directory tree.
///
/// PORT: Java parallelizes over files with an `AbstractQueueVisitor` on virtual threads; this
/// visits sequentially in readdir order. The result on disk is the same.
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:98 SandboxHelpers.DirectoryCopier
struct DirectoryCopier {
    source_root: Path,
    target_root: Path,
}

impl DirectoryCopier {
    // Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:112 SandboxHelpers.DirectoryCopier.run
    fn run(&self) -> io::Result<()> {
        DirectoryCopier::visit_directory(&self.source_root, &self.target_root)
    }

    // Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:121 SandboxHelpers.DirectoryCopier.visitDirectory
    fn visit_directory(source_dir: &Path, target_dir: &Path) -> io::Result<()> {
        let dirents = match source_dir.readdir(Symlinks::Nofollow) {
            Ok(d) => d,
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                // Make the source directory readable and try again (but only once).
                // Don't check the permissions upfront to optimize for the typical case.
                source_dir.chmod(0o755)?;
                source_dir.readdir(Symlinks::Nofollow)?
            }
            Err(e) => return Err(e),
        };
        target_dir.create_directory()?;
        for dirent in dirents {
            let source_child = source_dir.get_child(dirent.get_name());
            let target_child = target_dir.get_child(dirent.get_name());
            match dirent.get_type() {
                DirentType::Directory => {
                    DirectoryCopier::visit_directory(&source_child, &target_child)?
                }
                DirentType::File => copy_file(&source_child, &target_child)?,
                DirentType::Symlink => copy_symlink(&source_child, &target_child)?,
                DirentType::Unknown => {
                    return Err(io::Error::other(format!(
                        "Don't know how to copy {} to {}",
                        source_child, target_child
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Moves or copies all given outputs from a root to another.
///
/// Moves if possible, otherwise makes a copy. It is unspecified whether the source files still
/// exist after this method returns.
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:178 SandboxHelpers.moveOutputs
pub fn move_outputs(
    outputs: &SandboxOutputs,
    source_root: &Path,
    target_root: &Path,
) -> io::Result<()> {
    for (key, value) in outputs.files.iter().chain(outputs.dirs.iter()) {
        let source = source_root.get_relative(value);
        let target = target_root.get_relative(key);

        let Some(stat) = source.stat_if_found(Symlinks::Nofollow)? else {
            // The correct thing to do here would be to delete the target path.
            // Unfortunately, this breaks streamed test output, which causes the test log to be
            // written directly to the target path even when sandboxing is enabled. Until we either
            // fix streamed test output or create a way to reliably detect it, just skip the
            // deletion.
            continue;
        };

        // Delete the target if it already exists.
        // Some test spawn outputs aren't action outputs, so they aren't deleted before action
        // execution.
        target.delete_tree()?;

        // Create the target's parent directory if it doesn't already exist.
        // Some test spawn outputs aren't action outputs, so their parent directories aren't
        // created before action execution.
        target
            .get_parent_directory()
            .expect("NullPointerException: target has no parent")
            .create_directory_and_parents()?;

        // Prefer to move outputs through a rename, avoiding a more expensive copy.
        if source.rename_to(&target).is_err() {
            // Assume that the rename failed because it was cross-device.
            // TODO(tjgq): Distinguish a cross-device rename from other errors.
            // PORT: skipped, tracing (Java log) (on the first copy: "Moving files out of the sandbox (e.g.
            // from %s to %s) had to be done with a file copy, which is detrimental to
            // performance; are the two trees in different file systems?").
            let _first_copy = WARNED_ABOUT_MOVES_BEING_COPIES
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok();

            // Make a copy.
            // Do as little work as possible, as any overhead adds up for large trees. In
            // particular, avoid FileSystemUtils, which spends time deleting preexisting files and
            // preserving attributes: we know output directories start out empty, and don't care
            // about attributes. Don't delete the original; leave it to the sandbox to clean up
            // after itself.
            if stat.is_file() {
                copy_file(&source, &target)?;
            } else if stat.is_directory() {
                let copier = DirectoryCopier {
                    source_root: source.clone(),
                    target_root: target.clone(),
                };
                copier.run()?;
            } else if stat.is_symbolic_link() {
                copy_symlink(&source, &target)?;
            } else {
                return Err(io::Error::other(format!(
                    "Don't know how to copy {} into {} because it has an unsupported type",
                    source, target
                )));
            }
        }
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:240 SandboxHelpers.copyFile
fn copy_file(source: &Path, target: &Path) -> io::Result<()> {
    fn transfer(source: &Path, target: &Path) -> io::Result<()> {
        let mut input = source.get_input_stream()?;
        let mut out = target.get_output_stream()?;
        io::copy(&mut input, &mut out)?;
        out.flush()
    }
    match transfer(source, target) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            // Actions may create unreadable output files.
            // Make the source file readable and try again (but only once).
            // Don't check the permissions upfront to optimize for the typical case.
            source.chmod(0o644)?;
            transfer(source, target)
        }
        Err(e) => Err(e),
    }
}

// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:256 SandboxHelpers.copySymlink
fn copy_symlink(source: &Path, target: &Path) -> io::Result<()> {
    target.create_symbolic_link_to_fragment(&source.read_symbolic_link()?)
}

/// Cleans the existing sandbox at `root` to match the `inputs`, updating `inputs_to_create` and
/// `dirs_to_create` to not contain existing inputs and dirs. Existing directories or files that
/// are either not needed `inputs` or don't have the right content or symlink target path are
/// removed.
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:266 SandboxHelpers.cleanExisting
pub fn clean_existing(
    root: &Path,
    inputs: &SandboxInputs,
    inputs_to_create: &mut IndexSet<PathFragment>,
    dirs_to_create: &mut IndexSet<PathFragment>,
    work_dir: &Path,
    tree_deleter: Option<&dyn TreeDeleter>,
) -> io::Result<()> {
    clean_existing_with_contents(
        root,
        inputs,
        inputs_to_create,
        dirs_to_create,
        work_dir,
        tree_deleter,
        /* sandbox_contents= */ None,
    )
}

/// `cleanExisting(..., @Nullable SandboxContents sandboxContents)`.
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:284 SandboxHelpers.cleanExisting
pub fn clean_existing_with_contents(
    root: &Path,
    inputs: &SandboxInputs,
    inputs_to_create: &mut IndexSet<PathFragment>,
    dirs_to_create: &mut IndexSet<PathFragment>,
    work_dir: &Path,
    // PORT: Java's parameter is not annotated, but callers pass null (a `WorkerFactory` without a
    // tree deleter); `cleanRecursively` then deletes directly, and the in-memory variant fails
    // with a NullPointerException only when it has a tree to delete.
    tree_deleter: Option<&dyn TreeDeleter>,
    sandbox_contents: Option<&SandboxContents>,
) -> io::Result<()> {
    let inaccessible_helper_dir = work_dir.get_relative_str(INACCESSIBLE_HELPER_DIR);
    // Setting the permissions is necessary when we are using an asynchronous tree deleter in order
    // to move the directory first. This is not necessary for a synchronous tree deleter because
    // the permissions are only needed in the parent directory in that case.
    if inaccessible_helper_dir.exists() {
        inaccessible_helper_dir.set_executable(true)?;
        inaccessible_helper_dir.set_writable(true)?;
        inaccessible_helper_dir.set_readable(true)?;
    }

    // To avoid excessive scanning of dirsToCreate for prefix dirs, we prepopulate this set of
    // prefixes.
    let mut prefix_dirs: HashSet<PathFragment> = HashSet::new();
    for dir in dirs_to_create.iter() {
        check_interrupted(None)?;
        let mut parent = dir.get_parent_directory();
        while let Some(p) = parent {
            if prefix_dirs.contains(&p) {
                break;
            }
            prefix_dirs.insert(p.clone());
            parent = p.get_parent_directory();
        }
    }
    match sandbox_contents {
        None => clean_recursively(
            root,
            inputs,
            inputs_to_create,
            dirs_to_create,
            work_dir,
            &prefix_dirs,
            tree_deleter,
        ),
        Some(contents) => clean_recursively_with_in_memory_contents(
            root,
            inputs,
            inputs_to_create,
            dirs_to_create,
            work_dir,
            &prefix_dirs,
            tree_deleter,
            contents,
        ),
    }
}

/// Deletes unnecessary files/directories and updates the sets if something on disk is already
/// correct and doesn't need any changes.
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:336 SandboxHelpers.cleanRecursivelyWithInMemoryContents
fn clean_recursively_with_in_memory_contents(
    root: &Path,
    inputs: &SandboxInputs,
    inputs_to_create: &mut IndexSet<PathFragment>,
    dirs_to_create: &mut IndexSet<PathFragment>,
    work_dir: &Path,
    prefix_dirs: &HashSet<PathFragment>,
    tree_deleter: Option<&dyn TreeDeleter>,
    stash_contents: &SandboxContents,
) -> io::Result<()> {
    let execroot = work_dir
        .get_parent_directory()
        .expect("NullPointerException: workDir has no parent");
    for (name, value) in &stash_contents.file_map {
        check_interrupted(None)?;
        let abs_path = root.get_child(name);
        let path_relative_to_work_dir =
            get_path_relative_to_work_dir(&abs_path, work_dir, &execroot);
        let target_path = get_expected_symlink_target_path(&path_relative_to_work_dir, inputs);
        let should_keep = match value {
            None => matches!(inputs.files.get(&path_relative_to_work_dir), Some(None)),
            Some(v) => target_path.as_ref() == Some(v),
        };
        if should_keep {
            blaze_util::check_state!(inputs_to_create.shift_remove(&path_relative_to_work_dir));
        } else {
            abs_path.delete()?;
        }
    }
    for (name, contents) in &stash_contents.dir_map {
        check_interrupted(None)?;
        let abs_path = root.get_child(name);
        let path_relative_to_work_dir =
            get_path_relative_to_work_dir(&abs_path, work_dir, &execroot);
        if dirs_to_create.contains(&path_relative_to_work_dir)
            || prefix_dirs.contains(&path_relative_to_work_dir)
        {
            clean_recursively_with_in_memory_contents(
                &abs_path,
                inputs,
                inputs_to_create,
                dirs_to_create,
                work_dir,
                prefix_dirs,
                tree_deleter,
                contents,
            )?;
            dirs_to_create.shift_remove(&path_relative_to_work_dir);
        } else {
            tree_deleter
                .expect("NullPointerException: treeDeleter")
                .delete_tree(&abs_path)?;
        }
    }
    Ok(())
}

/// Deletes unnecessary files/directories and updates the sets if something on disk is already
/// correct and doesn't need any changes.
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:395 SandboxHelpers.cleanRecursively
fn clean_recursively(
    root: &Path,
    inputs: &SandboxInputs,
    inputs_to_create: &mut IndexSet<PathFragment>,
    dirs_to_create: &mut IndexSet<PathFragment>,
    work_dir: &Path,
    prefix_dirs: &HashSet<PathFragment>,
    tree_deleter: Option<&dyn TreeDeleter>,
) -> io::Result<()> {
    let execroot = work_dir
        .get_parent_directory()
        .expect("NullPointerException: workDir has no parent");
    for dirent in root.readdir(Symlinks::Nofollow)? {
        check_interrupted(None)?;
        let abs_path = root.get_child(dirent.get_name());
        let path_relative_to_work_dir = if abs_path.starts_with(work_dir) {
            // path is under workDir, i.e. execroot/<workspace name>. Simply get the relative path.
            abs_path.relative_to(work_dir)
        } else {
            // path is not under workDir, which means it belongs to one of external repositories
            // symlinked directly under execroot. Get the relative path based on there and prepend
            // it with the designated prefix, '../', so that it's still a valid relative path to
            // workDir.
            EXPERIMENTAL_EXTERNAL_PATH_PREFIX.get_relative(&abs_path.relative_to(&execroot))
        };
        let target_path = get_expected_symlink_target_path(&path_relative_to_work_dir, inputs);
        if let Some(target_path) = target_path {
            if dirent.get_type() == DirentType::Symlink
                && abs_path.read_symbolic_link()? == target_path
            {
                inputs_to_create.shift_remove(&path_relative_to_work_dir);
            } else if dirent.get_type() == DirentType::Directory {
                match tree_deleter {
                    // TODO(bazel-team): Use async tree deleter for workers too
                    None => abs_path.delete_tree()?,
                    Some(d) => d.delete_tree(&abs_path)?,
                }
            } else {
                abs_path.delete()?;
            }
        } else if dirent.get_type() == DirentType::Directory {
            if dirs_to_create.contains(&path_relative_to_work_dir)
                || prefix_dirs.contains(&path_relative_to_work_dir)
            {
                clean_recursively(
                    &abs_path,
                    inputs,
                    inputs_to_create,
                    dirs_to_create,
                    work_dir,
                    prefix_dirs,
                    tree_deleter,
                )?;
                dirs_to_create.shift_remove(&path_relative_to_work_dir);
            } else {
                match tree_deleter {
                    // TODO(bazel-team): Use async tree deleter for workers too
                    None => abs_path.delete_tree()?,
                    Some(d) => d.delete_tree(&abs_path)?,
                }
            }
        } else if !inputs_to_create.contains(&path_relative_to_work_dir) {
            abs_path.delete()?;
        }
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:458 SandboxHelpers.getPathRelativeToWorkDir
fn get_path_relative_to_work_dir(
    abs_path: &Path,
    work_dir: &Path,
    execroot: &Path,
) -> PathFragment {
    if abs_path.starts_with(work_dir) {
        // path is under workDir, i.e. execroot/<workspace name>. Simply get the relative path.
        abs_path.relative_to(work_dir)
    } else {
        // path is not under workDir, which means it belongs to one of external repositories
        // symlinked directly under execroot. Get the relative path based on there and prepend it
        // with the designated prefix, '../', so that it's still a valid relative path to workDir.
        EXPERIMENTAL_EXTERNAL_PATH_PREFIX.get_relative(&abs_path.relative_to(execroot))
    }
}

/// Returns what the target path of the symlink `path` should be according to `inputs`.
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:474 SandboxHelpers.getExpectedSymlinkTargetPath
fn get_expected_symlink_target_path(
    path: &PathFragment,
    inputs: &SandboxInputs,
) -> Option<PathFragment> {
    if let Some(Some(file)) = inputs.files.get(path) {
        return Some(file.as_fragment().clone());
    }
    inputs.symlinks.get(path).cloned()
}

/// Populates the provided sets with the inputs and directories that need to be created.
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:484 SandboxHelpers.populateInputsAndDirsToCreate
pub fn populate_inputs_and_dirs_to_create<'a>(
    writable_dirs: impl IntoIterator<Item = &'a PathFragment>,
    inputs_to_create: &mut IndexSet<PathFragment>,
    dirs_to_create: &mut IndexSet<PathFragment>,
    input_files: impl IntoIterator<Item = &'a PathFragment>,
    outputs: &SandboxOutputs,
) {
    // Add all worker files, input files, and the parent directories.
    for input in input_files {
        inputs_to_create.insert(input.clone());
        dirs_to_create.insert(
            input
                .get_parent_directory()
                .expect("NullPointerException: input has no parent"),
        );
    }

    // And all parent directories of output files. Note that we don't add the files themselves --
    // any pre-existing files that have the same path as an output should get deleted.
    for file in outputs.files.values() {
        dirs_to_create.insert(
            file.get_parent_directory()
                .expect("NullPointerException: output has no parent"),
        );
    }

    // Add all output directories.
    dirs_to_create.extend(outputs.dirs.values().cloned());

    // Add some directories that should be writable, and thus exist.
    dirs_to_create.extend(writable_dirs.into_iter().cloned());
}

/// Creates directory and all ancestors for it at a given path.
///
/// This method uses (and updates) the set of already known directories in order to minimize
/// the I/O involved with creating directories. For example a path of `1/2/3/4` created after
/// `1/2/3/5` only calls for creating `1/2/3/5`. We can use the set of known directories to
/// discover that `1/2/3` already exists instead of deferring to the filesystem for it.
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:518 SandboxHelpers.createDirectoryAndParentsInSandboxRoot
pub fn create_directory_and_parents_in_sandbox_root(
    path: &Path,
    known_directories: &mut HashSet<Path>,
    sandbox_exec_root: &Path,
) -> io::Result<()> {
    if known_directories.contains(path) {
        return Ok(());
    }
    let parent = path.get_parent_directory().unwrap_or_else(|| {
        panic!(
            "NullPointerException: Path {} is not under/siblings of sandboxExecRoot: {}",
            path, sandbox_exec_root
        )
    });
    create_directory_and_parents_in_sandbox_root(&parent, known_directories, sandbox_exec_root)?;
    path.create_directory()?;
    known_directories.insert(path.clone());
    Ok(())
}

/// Creates all directories needed for the sandbox.
///
/// No input can be a child of another input, because otherwise we might try to create a symlink
/// below another symlink we created earlier - which means we'd actually end up writing somewhere
/// in the workspace.
///
/// If all inputs were regular files, this situation could naturally not happen - but
/// unfortunately, we might get the occasional action that has directories in its inputs.
///
/// Creating all parent directories first ensures that we can safely create symlinks to
/// directories, too, because we'll get an IOException with EEXIST if inputs happen to be nested
/// once we start creating the symlinks for all inputs.
///
/// `strict`: if true, absolute directories or directories with multiple up-level references are
/// disallowed, for stricter sandboxing.
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:552 SandboxHelpers.createDirectories
pub fn create_directories<'a>(
    dirs_to_create: impl IntoIterator<Item = &'a PathFragment>,
    dir: &Path,
    strict: bool,
) -> io::Result<()> {
    let mut known_directories: HashSet<Path> = HashSet::new();
    // Add sandboxExecRoot and it's parent -- all paths must fall under the parent of
    // sandboxExecRoot and we know that sandboxExecRoot exists. This stops the recursion in
    // createDirectoryAndParentsInSandboxRoot.
    known_directories.insert(dir.clone());
    known_directories.insert(
        dir.get_parent_directory()
            .expect("NullPointerException: dir has no parent"),
    );
    known_directories.insert(get_tmp_dir_path(dir));

    for path in dirs_to_create {
        check_interrupted(None)?;
        if strict {
            blaze_util::check_argument!(!path.is_absolute(), "{}", path);
            if path.contains_uplevel_references() && path.is_multi_segment() {
                // Allow a single up-level reference to allow inputs from the siblings of the main
                // repository in the sandbox execution root, but forbid multiple up-level
                // references. PathFragment is normalized, so up-level references are guaranteed to
                // be at the beginning.
                blaze_util::check_argument!(
                    !PathFragment::contains_uplevel_references_str(path.get_segment(1)),
                    "{} escapes the sandbox exec root.",
                    path
                );
            }
        }

        create_directory_and_parents_in_sandbox_root(
            &dir.get_relative(path),
            &mut known_directories,
            dir,
        )?;
    }
    Ok(())
}

/// `createFailureDetail(message, detailedCode)`.
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:585 SandboxHelpers.createFailureDetail
pub fn create_failure_detail(message: &str, detailed_code: sandbox::Code) -> FailureDetail {
    FailureDetail {
        message: message.to_string(),
        category: Some(failure_detail::Category::Sandbox(
            blaze_protos::failure_details::Sandbox {
                code: detailed_code as i32,
            },
        )),
    }
}

/// Adds additional bind mounts entries from `paths` (source -> target) to `bind_mounts`
/// (target -> source).
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:593 SandboxHelpers.mountAdditionalPaths
pub fn mount_additional_paths(
    paths: &IndexMap<String, String>,
    sandbox_exec_root: &Path,
    bind_mounts: &mut BTreeMap<Path, Path>,
) -> Result<(), UserExecException> {
    let fs = sandbox_exec_root.get_file_system();
    for (key, value) in paths {
        // Java: FileSystem.getPath throws IllegalArgumentException for a relative path.
        let target_fragment = PathFragment::create(value);
        if !target_fragment.is_absolute() {
            let e_message = format!("Paths must be absolute: '{}'", target_fragment);
            return Err(UserExecException::new(create_failure_detail(
                &format!(
                    "Error occurred when analyzing bind mount pairs. {}",
                    e_message
                ),
                sandbox::Code::BindMountAnalysisFailure,
            )));
        }
        let mount_target = fs.get_path_fragment(&target_fragment);
        // If source path is relative, treat it as a relative path inside the execution root
        let mount_source = sandbox_exec_root.get_relative_str(key);
        // If a target has more than one source path, the latter one will take effect.
        bind_mounts.insert(mount_target, mount_source);
    }
    Ok(())
}

/// Wrapper class for the inputs of a sandbox (`SandboxHelpers.SandboxInputs`).
///
/// A `None` file value is an empty file (`VirtualActionInput.EMPTY_MARKER`).
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:614 SandboxHelpers.SandboxInputs
#[derive(Clone, Debug, Default)]
pub struct SandboxInputs {
    files: IndexMap<PathFragment, Option<Path>>,
    virtual_inputs: IndexMap<ActionInputRef, Vec<u8>>,
    // PORT: Java's values are nullable in principle but never null (processInputFiles reads them
    // from symlinks); the null checks in createContentMap/createInputs are dropped.
    symlinks: IndexMap<PathFragment, PathFragment>,
}

impl SandboxInputs {
    /// `new SandboxInputs(files, virtualInputs, symlinks)`.
    pub fn new(
        files: IndexMap<PathFragment, Option<Path>>,
        virtual_inputs: IndexMap<ActionInputRef, Vec<u8>>,
        symlinks: IndexMap<PathFragment, PathFragment>,
    ) -> SandboxInputs {
        SandboxInputs {
            files,
            virtual_inputs,
            symlinks,
        }
    }

    /// `getEmptyInputs()`.
    pub fn get_empty_inputs() -> SandboxInputs {
        SandboxInputs::default()
    }

    /// `getFiles()`.
    pub fn get_files(&self) -> &IndexMap<PathFragment, Option<Path>> {
        &self.files
    }

    /// `getSymlinks()`.
    pub fn get_symlinks(&self) -> &IndexMap<PathFragment, PathFragment> {
        &self.symlinks
    }

    /// `getVirtualInputDigests()`.
    pub fn get_virtual_input_digests(&self) -> &IndexMap<ActionInputRef, Vec<u8>> {
        &self.virtual_inputs
    }

    /// Returns a new instance with only the inputs/symlinks listed in `allowed` included.
    // Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:651 SandboxHelpers.SandboxInputs.limitedCopy
    pub fn limited_copy(&self, allowed: &HashSet<PathFragment>) -> SandboxInputs {
        SandboxInputs {
            files: self
                .files
                .iter()
                .filter(|(k, _)| allowed.contains(*k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            virtual_inputs: IndexMap::new(),
            symlinks: self
                .symlinks
                .iter()
                .filter(|(k, _)| allowed.contains(*k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
    }
}

/// Returns the inputs of a Spawn as a map of PathFragments relative to an execRoot to paths in
/// the host filesystem where the input files can be found.
///
/// Inputs nested under an input that isn't a symlink are omitted: a file can't contain other
/// paths, so such an input is a directory (e.g. a source directory artifact) that already provides
/// the nested paths. Creating them separately would require creating the directory input as a
/// real directory in the sandbox, which clashes with creating it as a symlink or copy. This
/// matches how `MerkleTreeComputer` stages such inputs for remote execution. Inputs nested under
/// an input symlink are retained and fail when the sandbox is created.
///
/// `input_map` is sorted by `PathFragment.HIERARCHICAL_COMPARATOR` (enforced by its key type).
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:681 SandboxHelpers.processInputFiles
pub fn process_input_files(
    input_map: &InputMapping,
    exec_root: &Path,
) -> io::Result<SandboxInputs> {
    // Java: TreeMap (natural PathFragment order).
    let mut input_files: BTreeMap<PathFragment, Option<Path>> = BTreeMap::new();
    let mut input_symlinks: BTreeMap<PathFragment, PathFragment> = BTreeMap::new();
    let mut virtual_inputs: IndexMap<ActionInputRef, Vec<u8>> = IndexMap::new();

    // The last input that isn't a symlink. Any inputs nested under it directly follow it.
    let mut last_file: Option<PathFragment> = None;
    for (key, action_input) in input_map {
        check_interrupted(None)?;
        let path_fragment = &key.0;
        if let Some(last) = &last_file
            && path_fragment.starts_with(last)
        {
            continue;
        }
        if let Some(input) = action_input.as_virtual_action_input() {
            let digest = input.atomically_write_relative_to(exec_root)?;
            virtual_inputs.insert(action_input.clone(), digest);
        }

        if action_input.is_symlink() {
            last_file = None;
            let input_path = exec_root.get_relative(action_input.get_exec_path());
            input_symlinks.insert(path_fragment.clone(), input_path.read_symbolic_link()?);
        } else {
            last_file = Some(path_fragment.clone());
            let is_empty_action_input = action_input
                .as_virtual_action_input()
                .is_some_and(|v| v.is_empty_action_input());
            let input_path = if is_empty_action_input {
                None
            } else {
                Some(exec_root.get_relative(action_input.get_exec_path()))
            };
            input_files.insert(path_fragment.clone(), input_path);
        }
    }
    Ok(SandboxInputs::new(
        input_files.into_iter().collect(),
        virtual_inputs,
        input_symlinks.into_iter().collect(),
    ))
}

/// The file and directory outputs of a sandboxed spawn (`SandboxHelpers.SandboxOutputs`).
///
/// `files`: a map from output file exec paths to paths in the sandbox. `dirs`: a map from output
/// directory exec paths to paths in the sandbox.
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:729 SandboxHelpers.SandboxOutputs
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SandboxOutputs {
    pub files: IndexMap<PathFragment, PathFragment>,
    pub dirs: IndexMap<PathFragment, PathFragment>,
}

impl SandboxOutputs {
    /// `create(ImmutableMap files, ImmutableMap dirs)`.
    // Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:740 SandboxHelpers.SandboxOutputs.create
    pub fn create(
        files: IndexMap<PathFragment, PathFragment>,
        dirs: IndexMap<PathFragment, PathFragment>,
    ) -> SandboxOutputs {
        SandboxOutputs { files, dirs }
    }

    /// `create(ImmutableSet files, ImmutableSet dirs)`: identity mappings.
    // Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:746 SandboxHelpers.SandboxOutputs.create
    pub fn create_from_sets(
        files: impl IntoIterator<Item = PathFragment>,
        dirs: impl IntoIterator<Item = PathFragment>,
    ) -> SandboxOutputs {
        SandboxOutputs {
            files: files.into_iter().map(|f| (f.clone(), f)).collect(),
            dirs: dirs.into_iter().map(|d| (d.clone(), d)).collect(),
        }
    }

    /// `getEmptyInstance()`.
    pub fn get_empty_instance() -> SandboxOutputs {
        SandboxOutputs::default()
    }

    /// `files()`.
    pub fn files(&self) -> &IndexMap<PathFragment, PathFragment> {
        &self.files
    }

    /// `dirs()`.
    pub fn dirs(&self) -> &IndexMap<PathFragment, PathFragment> {
        &self.dirs
    }
}

/// `getOutputs(Spawn)`.
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:758 SandboxHelpers.getOutputs
pub fn get_outputs(spawn: &dyn Spawn) -> SandboxOutputs {
    let mut files: IndexMap<PathFragment, PathFragment> = IndexMap::new();
    let mut dirs: IndexMap<PathFragment, PathFragment> = IndexMap::new();
    for output in spawn.get_output_files() {
        let mapped_path = spawn.get_path_mapper().map(output.get_exec_path());
        let is_tree = output.as_artifact().is_some_and(|a| a.is_tree_artifact());
        if is_tree {
            dirs.insert(output.get_exec_path().clone(), mapped_path);
        } else {
            files.insert(output.get_exec_path().clone(), mapped_path);
        }
    }
    SandboxOutputs::create(files, dirs)
}

/// Returns the path to the tmp directory of the given workDir of worker.
///
/// The structure of the worker directories should look like this: `<outputBase>/
/// |__bazel-workers/ |__worker-<id>-<mnemonic>/ |__worker-<id>-<mnemonic>-tmp/`
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:778 SandboxHelpers.getTmpDirPath
pub fn get_tmp_dir_path(work_dir: &Path) -> Path {
    let parent = work_dir
        .get_parent_directory()
        .expect("NullPointerException: workDir has no parent");
    parent
        .get_parent_directory()
        .expect("NullPointerException: workDir has no grandparent")
        .get_child(&format!("{}-tmp", parent.get_base_name()))
}

/// Returns true if the build options are set in a way that requires network access for all
/// actions. This is separate from `Spawns.requiresNetwork` to avoid having to keep a reference to
/// the full set of build options (and also for performance, since this only needs to be checked
/// once-per-build).
///
/// `test_arguments`: `buildOptions.getOptions(TestConfiguration.TestOptions.class).testArguments`.
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:792 SandboxHelpers.shouldAllowNetwork
pub fn should_allow_network(test_arguments: &[String]) -> bool {
    // Allow network access, when --java_debug is specified, otherwise we can't connect to the
    // remote debug server of the test. This intentionally overrides the "block-network" execution
    // tag.
    test_arguments
        .iter()
        .any(|a| a == "--wrapper_script_flag=--debug")
}

/// In-memory representation of the set of paths known to be present in a sandbox directory
/// (`SandboxHelpers.SandboxContents`).
///
/// Used to minimize the amount of I/O required to prepare a sandbox for reuse. The map keys are
/// individual path segments.
///
/// `file_map` maps names of known symlinks to their target paths and empty files to `None`;
/// `dir_map` maps names of known subdirectories to their contents.
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:812 SandboxHelpers.SandboxContents
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SandboxContents {
    pub file_map: HashMap<String, Option<PathFragment>>,
    pub dir_map: HashMap<String, SandboxContents>,
}

impl SandboxContents {
    /// `new SandboxContents()`.
    pub fn new() -> SandboxContents {
        SandboxContents::default()
    }

    /// `fileMap()`.
    pub fn file_map(&self) -> &HashMap<String, Option<PathFragment>> {
        &self.file_map
    }

    /// `dirMap()`.
    pub fn dir_map(&self) -> &HashMap<String, SandboxContents> {
        &self.dir_map
    }
}

/// A node of the flat `contentsMap` built by `createContentMap`. Java shares the
/// `SandboxContents` objects between the flat map and their parents' `dirMap`s; here the flat map
/// records the parent links and the tree is assembled at the end, which yields the same tree.
#[derive(Default)]
struct ContentsNode {
    file_map: HashMap<String, Option<PathFragment>>,
    // (name, path of the child node), in `putIfAbsent` order.
    children: IndexMap<String, PathFragment>,
}

/// Computes a [`SandboxContents`] for the filesystem hierarchy rooted at `work_dir`'s parent
/// directory, reflecting the expected inputs and outputs for a spawn.
///
/// This may be used in conjunction with [`update_content_map`] to speed up the sandbox setup for
/// a subsequent execution.
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:826 SandboxHelpers.createContentMap
pub fn create_content_map(
    work_dir: &Path,
    inputs: &SandboxInputs,
    outputs: &SandboxOutputs,
) -> SandboxContents {
    let mut contents_map: HashMap<PathFragment, ContentsNode> = HashMap::new();
    for (key, value) in &inputs.files {
        let parent = key
            .get_parent_directory()
            .expect("NullPointerException: input has no parent");
        let parent_was_present = !add_parent(&mut contents_map, &parent);
        contents_map
            .get_mut(&parent)
            .expect("present")
            .file_map
            .insert(
                key.get_base_name().to_string(),
                value.as_ref().map(|p| p.as_fragment().clone()),
            );
        add_all_parents(&mut contents_map, parent_was_present, parent);
    }
    for (key, value) in &inputs.symlinks {
        let parent = key
            .get_parent_directory()
            .expect("NullPointerException: input has no parent");
        let parent_was_present = !add_parent(&mut contents_map, &parent);
        contents_map
            .get_mut(&parent)
            .expect("present")
            .file_map
            .insert(key.get_base_name().to_string(), Some(value.clone()));
        add_all_parents(&mut contents_map, parent_was_present, parent);
    }

    let mut output_dirs: IndexSet<PathFragment> = IndexSet::new();
    for f in outputs.files.values() {
        output_dirs.insert(
            f.get_parent_directory()
                .expect("NullPointerException: output has no parent"),
        );
    }
    for d in outputs.dirs.values() {
        output_dirs.insert(d.clone());
    }
    for output_dir in output_dirs {
        let parent = output_dir;
        let parent_was_present = !add_parent(&mut contents_map, &parent);
        add_all_parents(&mut contents_map, parent_was_present, parent);
    }
    // TODO: Handle the sibling repository layout correctly. Currently, the code below assumes that
    // all paths descend from the main repository.
    let mut root = SandboxContents::new();
    let empty = PathFragment::empty_fragment();
    // PORT: Java puts `contentsMap.get(EMPTY_FRAGMENT)`, which is null for a spawn without inputs
    // and outputs (and later fails with a NullPointerException in the in-memory stash code); an
    // empty SandboxContents is put instead.
    let work_dir_contents = if contents_map.contains_key(&empty) {
        assemble_contents(&contents_map, &empty)
    } else {
        SandboxContents::new()
    };
    root.dir_map
        .insert(work_dir.get_base_name().to_string(), work_dir_contents);
    root
}

fn assemble_contents(
    contents_map: &HashMap<PathFragment, ContentsNode>,
    path: &PathFragment,
) -> SandboxContents {
    let node = &contents_map[path];
    let mut result = SandboxContents {
        file_map: node.file_map.clone(),
        dir_map: HashMap::new(),
    };
    for (name, child) in &node.children {
        result
            .dir_map
            .insert(name.clone(), assemble_contents(contents_map, child));
    }
    result
}

/// Updates a [`SandboxContents`] previously created by [`create_content_map`] to reflect any
/// filesystem modifications that occurred after the given timestamp.
///
/// This is necessary because an action may delete some of its inputs or create additional
/// declared outputs. We assume that a ctime check on directories is sufficient to detect such
/// modifications and avoid a full filesystem traversal.
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:875 SandboxHelpers.updateContentMap
pub fn update_content_map(
    root: &Path,
    timestamp: i64,
    stash_contents: &mut SandboxContents,
) -> io::Result<()> {
    if root.stat()?.get_last_change_time() > timestamp {
        let mut dirs_to_keep: HashSet<String> = HashSet::new();
        let mut files_and_symlinks_to_keep: HashSet<String> = HashSet::new();
        for dirent in root.readdir(Symlinks::Nofollow)? {
            check_interrupted(None)?;
            let name = dirent.get_name().to_string();
            let abs_path = root.get_child(&name);
            match dirent.get_type() {
                DirentType::Symlink => {
                    if matches!(stash_contents.file_map.get(&name), Some(Some(_)))
                        && abs_path.stat()?.get_last_change_time() <= timestamp
                    {
                        files_and_symlinks_to_keep.insert(name);
                    } else {
                        abs_path.delete()?;
                    }
                }
                DirentType::Directory => {
                    if let Some(child) = stash_contents.dir_map.get_mut(&name) {
                        dirs_to_keep.insert(name.clone());
                        update_content_map(&abs_path, timestamp, child)?;
                    } else {
                        abs_path.delete_tree()?;
                        stash_contents.dir_map.remove(&name);
                    }
                }
                DirentType::File => {
                    if matches!(stash_contents.file_map.get(&name), Some(None))
                        && abs_path.stat()?.get_last_change_time() <= timestamp
                    {
                        files_and_symlinks_to_keep.insert(name);
                    } else {
                        abs_path.delete()?;
                    }
                }
                DirentType::Unknown => {
                    abs_path.delete()?;
                }
            }
        }
        stash_contents
            .dir_map
            .retain(|k, _| dirs_to_keep.contains(k));
        stash_contents
            .file_map
            .retain(|k, _| files_and_symlinks_to_keep.contains(k));
    } else {
        for (name, contents) in stash_contents.dir_map.iter_mut() {
            let abs_path = root.get_child(name);
            update_content_map(&abs_path, timestamp, contents)?;
        }
    }
    Ok(())
}

/// `resetWarnedAboutMovesBeingCopiesForTesting()`.
pub fn reset_warned_about_moves_being_copies_for_testing() {
    WARNED_ABOUT_MOVES_BEING_COPIES.store(false, Ordering::SeqCst);
}

/// Returns true if `parent` was added (it was not present before).
// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:927 SandboxHelpers.addParent
fn add_parent(
    contents_map: &mut HashMap<PathFragment, ContentsNode>,
    parent: &PathFragment,
) -> bool {
    let mut parent_was_present = true;
    if !contents_map.contains_key(parent) {
        contents_map.insert(parent.clone(), ContentsNode::default());
        parent_was_present = false;
    }
    !parent_was_present
}

// Java: com/google/devtools/build/lib/sandbox/SandboxHelpers.java:937 SandboxHelpers.addAllParents
fn add_all_parents(
    contents_map: &mut HashMap<PathFragment, ContentsNode>,
    mut parent_was_present: bool,
    mut parent: PathFragment,
) {
    while !parent_was_present {
        let Some(grandparent) = parent.get_parent_directory() else {
            break;
        };
        if contents_map.contains_key(&grandparent) {
            parent_was_present = true;
        } else {
            contents_map.insert(grandparent.clone(), ContentsNode::default());
        }
        let grandparent_contents = contents_map.get_mut(&grandparent).expect("present");
        grandparent_contents
            .children
            .entry(parent.get_base_name().to_string())
            .or_insert_with(|| parent.clone());
        parent = grandparent;
    }
}
