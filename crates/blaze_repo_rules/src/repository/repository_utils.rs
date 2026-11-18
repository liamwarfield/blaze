//! Utility methods related to repo fetching.
//!
//! Java: com/google/devtools/build/lib/bazel/repository/RepositoryUtils.java.

use blaze_cmdline::Label;
use blaze_cmdline::label_constants::EXTERNAL_REPOSITORY_LOCATION;
use blaze_cmdline::label_constants::MODULE_DOT_BAZEL_FILE_NAME;
use blaze_cmdline::label_constants::REPO_FILE_NAME;
use blaze_cmdline::label_constants::WORKSPACE_DOT_BAZEL_FILE_NAME;
use blaze_cmdline::label_constants::WORKSPACE_FILE_NAME;
use blaze_repo_fetch::fs_utils;
use blaze_sky_loading::directories::LoadingDirectories;
use blaze_sky_loading::io::IOException;
use blaze_sky_loading::package_lookup::PackageLookupFunction;
use blaze_sky_loading::package_lookup::PackageLookupKey;
use blaze_sky_loading::package_lookup::PackageLookupValue;
use blaze_skyframe::Environment;
use blaze_skyframe::Interrupted;
use blaze_vfs::Path;
use blaze_vfs::PathFragment;
use blaze_vfs::RootedPath;
use blaze_vfs::Symlinks;

// Java: com/google/devtools/build/lib/bazel/repository/RepositoryUtils.java:41 RepositoryUtils.WORKSPACE_SYMLINK_NAME
pub const WORKSPACE_SYMLINK_NAME: &str = "_main";

// Java: com/google/devtools/build/lib/bazel/repository/RepositoryUtils.java:45 RepositoryUtils.isValidRepoRoot
pub fn is_valid_repo_root(directory: &Path) -> bool {
    // Keep in sync with //src/main/cpp/workspace_layout.h
    directory
        .get_relative(&WORKSPACE_DOT_BAZEL_FILE_NAME)
        .exists()
        || directory.get_relative(&WORKSPACE_FILE_NAME).exists()
        || directory.get_relative(&MODULE_DOT_BAZEL_FILE_NAME).exists()
        || directory.get_relative(&REPO_FILE_NAME).exists()
}

/// The rooted path of the file `label` names, or `Ok(Ok(None))` if a Skyframe value is missing.
/// `Ok(Err(message))` is Java's `EvalException`.
// Java: com/google/devtools/build/lib/bazel/repository/RepositoryUtils.java:54 RepositoryUtils.getRootedPathFromLabel
pub fn get_rooted_path_from_label(
    label: &Label,
    env: &mut Environment<'_>,
) -> Result<Result<Option<RootedPath>, String>, Interrupted> {
    let pkg_sky_key = PackageLookupKey::key(label.get_package_identifier().clone());
    let Some(pkg_lookup_value) = env.get_value(pkg_sky_key)? else {
        return Ok(Ok(None));
    };
    if !pkg_lookup_value.package_exists() {
        let mut message = pkg_lookup_value.get_error_msg();
        if matches!(*pkg_lookup_value, PackageLookupValue::NoBuildFile) {
            message = PackageLookupFunction::explain_no_build_file_value(
                label.get_package_identifier(),
                env,
            )?;
        }
        return Ok(Err(format!(
            "Unable to load package for {}: {}",
            label, message
        )));
    }

    // And now for the file
    let package_root = pkg_lookup_value.get_root().clone();
    Ok(Ok(Some(RootedPath::to_rooted_path(
        package_root,
        &label.to_path_fragment(),
    ))))
}

// Java: com/google/devtools/build/lib/bazel/repository/RepositoryUtils.java:74 RepositoryUtils.getExternalRepositoryDirectory
pub fn get_external_repository_directory(directories: &LoadingDirectories) -> Path {
    directories
        .get_output_base()
        .get_relative(&EXTERNAL_REPOSITORY_LOCATION)
}

/// The result of [`replant_symlinks`].
///
/// `safe_for_local_cache`: whether the repo is safe to cache in the local repo contents cache
/// after replanting. `safe_for_remote_cache`: whether the repo is safe to cache in the remote
/// repo contents cache after replanting.
// Java: com/google/devtools/build/lib/bazel/repository/RepositoryUtils.java:86 RepositoryUtils.ReplantSymlinksResult
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplantSymlinksResult {
    pub safe_for_local_cache: bool,
    pub safe_for_remote_cache: bool,
}

impl ReplantSymlinksResult {
    pub fn safe_for_local_cache(&self) -> bool {
        self.safe_for_local_cache
    }

    pub fn safe_for_remote_cache(&self) -> bool {
        self.safe_for_remote_cache
    }
}

/// Returns all of the paths below `root` (not following symlinks) for which `predicate` is true,
/// in directory-listing order, depth first.
///
/// PORT: move to blaze_vfs (Java `FileSystemUtils.traverseTree`).
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:536 FileSystemUtils.traverseTree
fn traverse_tree(root: &Path, predicate: &dyn Fn(&Path) -> bool) -> std::io::Result<Vec<Path>> {
    let mut paths = Vec::new();
    traverse_tree_into(&mut paths, root, predicate)?;
    Ok(paths)
}

// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:550 FileSystemUtils.traverseTree
fn traverse_tree_into(
    paths: &mut Vec<Path>,
    root: &Path,
    predicate: &dyn Fn(&Path) -> bool,
) -> std::io::Result<()> {
    for p in root.get_directory_entries()? {
        if predicate(&p) {
            paths.push(p.clone());
        }
        if p.is_directory_with(Symlinks::Nofollow) {
            traverse_tree_into(paths, &p, predicate)?;
        }
    }
    Ok(())
}

/// Replants the symlinks under the specified repository directory.
///
/// Re-writes symlinks that originally point to a path under the external root to relative paths.
/// Same-repo symlinks (those pointing back into `repo_dir`) are replanted to repo-relative paths
/// that work regardless of the repo's physical location. Cross-repo symlinks are replanted to
/// relative paths going through `repo_dir_parent_to_external_root`.
///
/// - `repo_dir`: the path to the repository directory.
/// - `workspace`: the path to the workspace directory.
/// - `external_repo_root`: the path to the root of external repositories.
/// - `repo_dir_parent_to_external_root`: the relative path from the parent directory of
///   `repo_dir` to the external root (or a symlink to it), which is used to rewrite cross-repo
///   symlink targets to relative paths.
/// - `replant_symlinks_into_main_repo`: if true, symlinks pointing into the main repo are treated
///   like cross-repo symlinks by routing them through a symlink to the workspace planted under the
///   external root (used by vendoring, where the vendored repo has to keep working from a different
///   checkout of the workspace). If false, such symlinks are left untouched.
// Java: com/google/devtools/build/lib/bazel/repository/RepositoryUtils.java:109 RepositoryUtils.replantSymlinks
pub fn replant_symlinks(
    repo_dir: &Path,
    workspace: &Path,
    external_repo_root: &Path,
    repo_dir_parent_to_external_root: &PathFragment,
    replant_symlinks_into_main_repo: bool,
) -> Result<ReplantSymlinksResult, IOException> {
    let mut portable_symlinks_only = true;
    // TODO(#30160): Repos with symlinks pointing out of the repo are currently excluded from the
    // remote repo contents cache since cross-FS resolution of symlinks proved tricky to get right.
    let mut symlinks_resolve_within_repo = true;
    let result = (|| -> std::io::Result<()> {
        let symlinks = traverse_tree(repo_dir, &|p: &Path| p.is_symbolic_link())?;
        let workspace_symlink_under_external = external_repo_root.get_child(WORKSPACE_SYMLINK_NAME);
        if replant_symlinks_into_main_repo {
            fs_utils::ensure_symbolic_link(
                &workspace_symlink_under_external,
                workspace.as_fragment(),
            )?;
        }
        for symlink in &symlinks {
            let mut target = symlink.read_symbolic_link()?;
            // PORT: `originalTarget` is only used by the Windows branch.
            if target.starts_with(workspace.as_fragment()) {
                symlinks_resolve_within_repo = false;
                if !replant_symlinks_into_main_repo {
                    // Symlinks pointing into the main repo can't be replanted to a relative path
                    // that stays under the external root. They make the repo unsafe to cache.
                    portable_symlinks_only = false;
                    continue;
                }
                target = workspace_symlink_under_external
                    .as_fragment()
                    .get_relative(&target.relative_to(workspace.as_fragment()));
            }
            if !target.is_absolute() {
                // A symlink that was created with a relative target by the repo rule. Resolve it
                // against the symlink's parent directory to determine whether it stays within the
                // repo.
                let parent = symlink
                    .get_parent_directory()
                    .expect("a symlink under the repo has a parent");
                if !parent.get_relative(&target).starts_with(repo_dir) {
                    portable_symlinks_only = false;
                    symlinks_resolve_within_repo = false;
                }
                continue;
            }
            if !target.starts_with(external_repo_root.as_fragment()) {
                // This symlink doesn't point into any Bazel repo, including the main repo, and
                // thus its target isn't managed by Bazel. We assume such symlinks are portable
                // across machines on which the repo is relevant (e.g. /lib/ld-linux.so* or
                // /usr/bin/ld)
                symlinks_resolve_within_repo = false;
                continue;
            }
            let new_target = if target.starts_with(repo_dir.as_fragment()) {
                // Same-repo symlink: replant relative within the repo. This is always safe
                // regardless of where the repo is physically located.
                let target_relative_to_repo = target.relative_to(repo_dir.as_fragment());
                let depth = symlink.relative_to(repo_dir).segment_count() - 1;
                PathFragment::create(&"../".repeat(depth)).get_relative(&target_relative_to_repo)
            } else {
                // Cross-repo symlink: replant through the parent directory. On Unix, these
                // relative paths only resolve correctly when the repo is at its original location
                // (under the external root), not when moved to a shared cache. This is because
                // symlink resolution is based on physical paths. On Windows, where symlinks are
                // resolved using logical paths, it may be possible to use these symlinks
                // portably, but this would likely require changes to FileFunction to mimic this
                // resolution behavior.
                portable_symlinks_only = false;
                symlinks_resolve_within_repo = false;
                // Rewrite for consistency even if not portable. A mix of absolute and relative
                // symlinks would result in less predictable behavior and reduced test coverage.
                let repo_dir_parent = repo_dir
                    .get_parent_directory()
                    .expect("the repo directory has a parent");
                PathFragment::create(
                    &"../".repeat(symlink.relative_to(&repo_dir_parent).segment_count() - 1),
                )
                .get_relative(repo_dir_parent_to_external_root)
                .get_relative(&target.relative_to(external_repo_root.as_fragment()))
            };
            // PORT: skipped, Windows only (Files.createSymbolicLink branch).
            fs_utils::ensure_symbolic_link(symlink, &new_target)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        return Err(IOException::new(format!(
            "Failed to rewrite symlinks under {}: {}",
            repo_dir,
            IOException::from_io(&e).get_message()
        )));
    }
    Ok(ReplantSymlinksResult {
        safe_for_local_cache: portable_symlinks_only,
        safe_for_remote_cache: symlinks_resolve_within_repo,
    })
}
