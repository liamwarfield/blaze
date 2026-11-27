//! Port of `lib/exec/SymlinkTreeHelper.java`.

use std::collections::BTreeMap;
use std::io;

use blaze_actions::actions::artifact::Artifact;
use blaze_protos::failure_details::execution::Code;
use blaze_util::profiler::{Profiler, ProfilerTask};
use blaze_vfs::{DirentType, Path, PathFragment, Symlinks};

use crate::actions::ExecException;
use crate::actions::FilesetOutputSymlink;
use crate::actions::action_adapters::as_blaze_artifact;
use crate::analysis::actions::environmental_exec_exception;
use crate::analysis::actions::file_system_utils;

/// Helper class responsible for the symlink tree creation. Used to generate runfiles and fileset
/// symlink farms.
// Java: com/google/devtools/build/lib/exec/SymlinkTreeHelper.java:41 SymlinkTreeHelper
#[derive(Debug, Clone)]
pub struct SymlinkTreeHelper {
    input_manifest: Path,
    output_manifest: Path,
    symlink_tree_root: Path,
    workspace_name: String,
}

/// Obtains a symlink target path from a `T`.
// Java: com/google/devtools/build/lib/exec/SymlinkTreeHelper.java:76 SymlinkTreeHelper.TargetPathFunction
// Java: com/google/devtools/build/lib/exec/SymlinkTreeHelper.java:78 SymlinkTreeHelper.TargetPathFunction.get
type TargetPathFunction<'a, T> = &'a dyn Fn(&T) -> io::Result<PathFragment>;

impl SymlinkTreeHelper {
    /// Creates SymlinkTreeHelper instance. Can be used independently of SymlinkTreeAction.
    ///
    /// - `input_manifest`: exec path to the input runfiles manifest
    /// - `output_manifest`: path to the output runfiles manifest
    /// - `symlink_tree_root`: the root of the symlink tree to be created
    /// - `workspace_name`: the name of the workspace, used to create the workspace subdirectory
    // Java: com/google/devtools/build/lib/exec/SymlinkTreeHelper.java:56 SymlinkTreeHelper.<init>
    pub fn new(
        input_manifest: &Path,
        output_manifest: &Path,
        symlink_tree_root: &Path,
        workspace_name: impl Into<String>,
    ) -> SymlinkTreeHelper {
        // Do not indirect paths through overlay filesystems (such as the action filesystem or a
        // snapshotting filesystem), for a few reasons:
        // (1) we always want to create the symlinks on disk, even if the overlay filesystem
        //     creates them in memory (at the time of writing, no action filesystem
        //     implementations do so, but this may change in the future).
        // (2) current action filesystem implementations are not a true overlay filesystem, so
        //     errors might occur in an incremental build when the parent directory of a symlink
        //     exists on disk but not in memory (see https://github.com/bazelbuild/bazel/issues/24867).
        // (3) Changes made to a file referenced by a symlink tree should be reflected in the
        //     symlink tree without having to rebuild. Therefore, if a snapshotting file system is
        //     used, we must use the underlying non-snapshotting file system instead to create the
        //     symlink tree.
        SymlinkTreeHelper {
            input_manifest: input_manifest.for_host_file_system(),
            output_manifest: output_manifest.for_host_file_system(),
            symlink_tree_root: symlink_tree_root.for_host_file_system(),
            workspace_name: workspace_name.into(),
        }
    }

    /// Creates a symlink tree for a fileset by making VFS calls.
    // Java: com/google/devtools/build/lib/exec/SymlinkTreeHelper.java:82 SymlinkTreeHelper.createFilesetSymlinks
    pub fn create_fileset_symlinks(
        &self,
        symlink_map: &BTreeMap<PathFragment, Option<PathFragment>>,
    ) -> Result<(), ExecException> {
        self.create_symlinks(
            symlink_map.iter().map(|(k, v)| (k.clone(), v.clone())),
            &|path: &PathFragment| Ok(path.clone()),
        )
    }

    /// Creates a symlink tree for a runfiles by making VFS calls.
    // Java: com/google/devtools/build/lib/exec/SymlinkTreeHelper.java:88 SymlinkTreeHelper.createRunfilesSymlinks
    pub fn create_runfiles_symlinks(
        &self,
        symlinks: &BTreeMap<PathFragment, Option<Artifact>>,
    ) -> Result<(), ExecException> {
        self.create_symlinks(
            symlinks.iter().map(|(k, v)| (k.clone(), v.clone())),
            &|artifact: &Artifact| {
                if artifact.is_symlink() {
                    // Unresolved symlinks are created textually.
                    artifact.get_path().read_symbolic_link()
                } else {
                    Ok(artifact.get_path().as_fragment().clone())
                }
            },
        )
    }

    /// Creates a symlink tree.
    // Java: com/google/devtools/build/lib/exec/SymlinkTreeHelper.java:98 SymlinkTreeHelper.createSymlinks
    fn create_symlinks<T>(
        &self,
        symlink_map: impl IntoIterator<Item = (PathFragment, Option<T>)>,
        target_path_fn: TargetPathFunction<'_, T>,
    ) -> Result<(), ExecException> {
        // Our strategy is to minimize mutating file system operations as much as possible.
        // Ideally, if there is an existing symlink tree with the expected contents, we don't make
        // any changes. Our algorithm goes as follows:
        //
        // 1. Create a tree structure that represents the entire set of paths that we want to
        //    exist. The tree structure contains a node for every intermediate directory. For
        //    example, this is the tree structure corresponding to the symlinks
        //    {"a/b/c": "foobar", "a/d/e": null}:
        //
        //      / b - c (symlink to "foobar")
        //    a
        //      \ d - e (empty file)
        //
        //    Note that we need to distinguish directories, symlinks, and empty files. In the
        //    Directory class below, we use two maps for that purpose: one for directories, and one
        //    for symlinks and empty files. This avoids having to create additional classes /
        //    objects to distinguish them.
        //
        // 2. Perform a depth-first traversal over the on-disk file system tree and make each
        //    directory match our expected directory layout. To that end, call readdir, and compare
        //    the result with the contents of the corresponding node in the in-memory tree.
        //
        //    For each Dirent entry in the readdir result:
        //    - If the entry is not in the current node, if the entry has an incompatible type, or
        //      if it is a symlink that points to the wrong location, delete the entry on disk
        //      (recursively).
        //    - Otherwise:
        //      - If the entry is a directory, recurse into that directory
        //      - In all cases, delete the entry in the current in-memory node.
        //
        // 3. For every remaining entry in the node, create the corresponding file, symlink, or
        //    directory on disk. If it is a directory, recurse into that directory.
        // Java: com/google/devtools/build/lib/exec/SymlinkTreeHelper.java:131 SymlinkTreeHelper.createSymlinks
        let _c = Profiler::instance().profile(ProfilerTask::INFO, "Create symlink tree");
        let run = || -> io::Result<()> {
            let mut root: Directory<T> = Directory::new();
            for (path, value) in symlink_map {
                blaze_util::check_argument!(!path.is_empty() && !path.is_absolute(), "{}", path);
                // This creates intermediate directory nodes as a side effect.
                let parent = path
                    .get_parent_directory()
                    .unwrap_or_else(PathFragment::empty_fragment);
                let parent_dir = root.walk(&parent);
                parent_dir.add_symlink(path.get_base_name().to_string(), value);
            }
            root.sync_tree_recursively(&self.symlink_tree_root, target_path_fn)
        };
        run()
            .map_err(|e| environmental_exec_exception(&e, Code::SymlinkTreeCreationIoException))?;
        self.create_workspace_subdirectory()
    }

    /// Creates a minimal runfiles directory containing only the output manifest and the
    /// workspace subdirectory. This is the expected state with --noenable_runfiles or
    /// --nobuild_runfile_links.
    // Java: com/google/devtools/build/lib/exec/SymlinkTreeHelper.java:149 SymlinkTreeHelper.createMinimalRunfilesDirectory
    pub fn create_minimal_runfiles_directory(&self) -> Result<(), ExecException> {
        self.clear_runfiles_directory()?;
        self.link_manifest()?;
        self.create_workspace_subdirectory()
    }

    /// Deletes the contents of the runfiles directory.
    // Java: com/google/devtools/build/lib/exec/SymlinkTreeHelper.java:156 SymlinkTreeHelper.clearRunfilesDirectory
    fn clear_runfiles_directory(&self) -> Result<(), ExecException> {
        // Java: com/google/devtools/build/lib/exec/SymlinkTreeHelper.java:160 SymlinkTreeHelper.clearRunfilesDirectory
        let _c = Profiler::instance().profile(ProfilerTask::INFO, "Clear symlink tree");
        self.symlink_tree_root
            .delete_trees_below()
            .map_err(|e| environmental_exec_exception(&e, Code::SymlinkTreeDeletionIoException))
    }

    /// Links the output manifest to the input manifest.
    // Java: com/google/devtools/build/lib/exec/SymlinkTreeHelper.java:165 SymlinkTreeHelper.linkManifest
    pub fn link_manifest(&self) -> Result<(), ExecException> {
        // TODO(b/443703909): This should probably be a copy, not a symlink.
        let run = || -> io::Result<()> {
            self.output_manifest.delete()?;
            self.output_manifest
                .create_symbolic_link(&self.input_manifest)
        };
        run()
            .map_err(|e| environmental_exec_exception(&e, Code::SymlinkTreeManifestLinkIoException))
    }

    // Java: com/google/devtools/build/lib/exec/SymlinkTreeHelper.java:175 SymlinkTreeHelper.createWorkspaceSubdirectory
    pub fn create_workspace_subdirectory(&self) -> Result<(), ExecException> {
        // Always create the subdirectory corresponding to the workspace (i.e., the main
        // repository). This is required by tests as their working directory, even with
        // --noenable_runfiles. But if the test action creates the directory and then proceeds to
        // execute the test spawn, this logic would remove it. For the sake of consistency, always
        // create the directory instead.
        self.symlink_tree_root
            .get_relative_str(&self.workspace_name)
            .create_directory()
            .map(|_| ())
            .map_err(|e| environmental_exec_exception(&e, Code::SymlinkTreeCreationIoException))
    }

    /// Processes a list of fileset symlinks into a map that can be passed to
    /// `OutputService.createSymlinkTree`.
    ///
    /// By convention, all symlinks are placed under a directory with the given workspace name.
    // Java: com/google/devtools/build/lib/exec/SymlinkTreeHelper.java:193 SymlinkTreeHelper.processFilesetLinks
    pub fn process_fileset_links(
        links: &[FilesetOutputSymlink],
        workspace_name: &str,
    ) -> BTreeMap<PathFragment, PathFragment> {
        let root = PathFragment::create(workspace_name);
        let mut symlinks = BTreeMap::new();
        for symlink in links {
            // symlink.target().getPath().asFragment()
            let target = match as_blaze_artifact(&symlink.target) {
                Some(a) => a.get_path().as_fragment().clone(),
                None => blaze_util::unported!(
                    "SymlinkTreeHelper.processFilesetLinks (unreachable in Bazel: only the non-open-sourced fileset rule makes fileset artifacts, packages/OutputFile.java:123)",
                    "fileset target {} is not a blaze_actions Artifact",
                    symlink.target.0.get_exec_path()
                ),
            };
            let name = root.get_relative(&symlink.name);
            // ImmutableMap.Builder.buildOrThrow: duplicate keys throw.
            blaze_util::check_argument!(
                symlinks.insert(name.clone(), target).is_none(),
                "Multiple entries with same key: {}",
                name
            );
        }
        // Fileset links are already deduplicated by name in SkyframeFilesetManifestAction.
        symlinks
    }
}

/// A node of the in-memory tree of `createSymlinks`.
///
/// PORT: Java's `HashMap`s only decide the order in which entries are created on disk, which is
/// not observable in the result; `BTreeMap` makes it deterministic.
// Java: com/google/devtools/build/lib/exec/SymlinkTreeHelper.java:205 SymlinkTreeHelper.Directory
struct Directory<T> {
    symlinks: BTreeMap<String, Option<T>>,
    directories: BTreeMap<String, Directory<T>>,
}

impl<T> Directory<T> {
    fn new() -> Directory<T> {
        Directory {
            symlinks: BTreeMap::new(),
            directories: BTreeMap::new(),
        }
    }

    // Java: com/google/devtools/build/lib/exec/SymlinkTreeHelper.java:209 Directory.addSymlink
    fn add_symlink(&mut self, basename: String, target: Option<T>) {
        self.symlinks.insert(basename, target);
    }

    // Java: com/google/devtools/build/lib/exec/SymlinkTreeHelper.java:213 Directory.walk
    fn walk(&mut self, dir: &PathFragment) -> &mut Directory<T> {
        let mut result = self;
        for segment in dir.segments() {
            result = result
                .directories
                .entry(segment.to_string())
                .or_insert_with(Directory::new);
        }
        result
    }

    // Java: com/google/devtools/build/lib/exec/SymlinkTreeHelper.java:221 Directory.syncTreeRecursively
    fn sync_tree_recursively(
        mut self,
        at: &Path,
        target_path_fn: TargetPathFunction<'_, T>,
    ) -> io::Result<()> {
        match at.stat_if_found(Symlinks::Follow)? {
            None => at.create_directory_and_parents()?,
            Some(stat) if !stat.is_directory() => {
                at.delete_tree()?;
                at.create_directory_and_parents()?;
            }
            Some(stat) => {
                // If the directory already exists, ensure it has appropriate permissions.
                // PORT: UnixFileStatus.getPermissions is never -1.
                let perms = stat.get_permissions();
                if (perms & 0o700) != 0o700 {
                    at.chmod(stat.get_permissions() | 0o700)?;
                }
            }
        }
        for dirent in at.readdir(Symlinks::Nofollow)? {
            let basename = dirent.get_name().to_string();
            let next = at.get_child(&basename);
            if let Some(value) = self.symlinks.remove(&basename) {
                match value {
                    None => {
                        if dirent.get_type() != DirentType::File {
                            next.delete_tree()?;
                            file_system_utils::create_empty_file(&next)?;
                        }
                        // For historical reasons, we don't truncate the file if one exists.
                        // TODO(tjgq): Ponder whether this is still necessary to preserve the
                        // intentional non-hermeticity of symlink trees under source edits.
                    }
                    Some(value) => {
                        // ensureSymbolicLink will replace a symlink that doesn't have the correct
                        // target, but everything else needs to be deleted first.
                        if dirent.get_type() != DirentType::Symlink {
                            next.delete_tree()?;
                        }
                        file_system_utils::ensure_symbolic_link(&next, &target_path_fn(&value)?)?;
                    }
                }
            } else if let Some(next_dir) = self.directories.remove(&basename) {
                if dirent.get_type() != DirentType::Directory {
                    next.delete_tree()?;
                }
                next_dir.sync_tree_recursively(&at.get_child(&basename), target_path_fn)?;
            } else {
                at.get_child(&basename).delete_tree()?;
            }
        }

        for (name, value) in std::mem::take(&mut self.symlinks) {
            let next = at.get_child(&name);
            match value {
                None => file_system_utils::create_empty_file(&next)?,
                Some(value) => {
                    file_system_utils::ensure_symbolic_link(&next, &target_path_fn(&value)?)?
                }
            }
        }
        for (name, dir) in std::mem::take(&mut self.directories) {
            dir.sync_tree_recursively(&at.get_child(&name), target_path_fn)?;
        }
        Ok(())
    }
}
