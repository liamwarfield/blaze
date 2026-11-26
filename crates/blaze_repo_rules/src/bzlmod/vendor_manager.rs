//! `VendorManager.vendorRepos`: moving (or copying, from the repo contents cache) fetched
//! repositories into the vendor directory.
//!
//! PORT: the registry half of `bazel/bzlmod/VendorManager.java` is
//! `blaze_bzlmod_model::vendor_manager::VendorManager`, which the index registry uses; this half
//! needs `RepositoryUtils.replantSymlinks` and the repo contents cache, which are this crate's.

use std::io;

use blaze_bzlmod_model::vendor_manager::{EXTERNAL_ROOT_SYMLINK_NAME, VendorManager};
use blaze_cmdline::RepositoryName;
use blaze_repo_fetch::cache::local_repo_contents_cache::RECORDED_INPUTS_SUFFIX;
use blaze_repo_fetch::fs_utils;
use blaze_vfs::{Path, PathFragment, Symlinks};

use crate::repository::repository_utils::replant_symlinks;

/// Vendors the specified repositories under the vendor directory.
///
/// TODO(pcloudy): Parallelize vendoring repos
///
/// - `external_repo_root`: the root directory of the external repositories.
/// - `workspace`: the workspace directory.
/// - `repos_to_vendor`: the list of repositories to vendor.
// Java: com/google/devtools/build/lib/bazel/bzlmod/VendorManager.java:67 VendorManager.vendorRepos
pub fn vendor_repos(
    vendor_manager: &VendorManager,
    external_repo_root: &Path,
    workspace: &Path,
    repos_to_vendor: &[RepositoryName],
) -> io::Result<()> {
    let vendor_directory = Path::create(
        vendor_manager.get_vendor_directory().clone(),
        workspace.get_file_system(),
    );
    if !vendor_directory.exists() {
        vendor_directory.create_directory_and_parents()?;
    }

    for repo in repos_to_vendor {
        // PORT: skipped, profiling (ProfilerTask.REPOSITORY_VENDOR).
        let repo_under_external = external_repo_root.get_child(repo.get_name());
        let repo_under_vendor = vendor_directory.get_child(repo.get_name());
        // This could happen when running the vendor command twice without changing anything.
        if repo_under_external.is_symbolic_link()
            && repo_under_external.resolve_symbolic_links()? == repo_under_vendor
        {
            continue;
        }

        let marker_under_external = external_repo_root.get_child(&repo.get_marker_file_name());
        let marker_under_vendor = vendor_directory.get_child(&repo.get_marker_file_name());
        // If the marker file doesn't exist under outputBase/external, then the repo is either
        // local (which cannot be in this case since local repos aren't vendored) or in the repo
        // contents cache.
        let is_cached = !marker_under_external.exists();
        let actual_marker_file = if is_cached {
            let cache_repo_dir = repo_under_external.resolve_symbolic_links()?;
            let name = format!(
                "{}{}",
                cache_repo_dir.get_base_name(),
                RECORDED_INPUTS_SUFFIX
            );
            cache_repo_dir
                .replace_name(&name)
                .expect("NullPointerException: replaceName")
        } else {
            marker_under_external.clone()
        };

        // At this point, the repo should exist under external dir, but check if the vendor src
        // is already up-to-date.
        if is_repo_up_to_date(&marker_under_vendor, &actual_marker_file)? {
            continue;
        }

        // Actually vendor the repo. If the repo is cached, copy it; otherwise move it.
        // 1. Clean up existing marker file and vendor dir.
        marker_under_vendor.delete()?;
        repo_under_vendor.delete_tree()?;
        repo_under_vendor.create_directory()?;
        // 2. Copy/move the marker file to a temporary one under vendor dir.
        let temporary_marker =
            vendor_directory.get_child(&format!("{}.tmp", repo.get_marker_file_name()));
        if is_cached {
            fs_utils::copy_file(&actual_marker_file, &temporary_marker)?;
        } else {
            fs_utils::move_file(&actual_marker_file, &temporary_marker)?;
        }
        // 3. Move/copy the external repo to vendor dir. Note that, in the "move" case, it's fine
        // if this step fails or is interrupted, because the marker file under external is gone
        // anyway.
        if is_cached {
            copy_trees_below(
                &repo_under_external.resolve_symbolic_links()?,
                &repo_under_vendor,
            )?;
        } else if repo_under_external.rename_to(&repo_under_vendor).is_err() {
            fs_utils::move_trees_below(&repo_under_external, &repo_under_vendor)?;
        }
        // 4. Re-plant symlinks pointing to a Bazel-managed path to a relative path to make sure
        // the vendor src keep working after being moved (including to a different checkout of
        // the workspace) or used with a different output base. We assume that a given vendor
        // directory is only used with one output base at a time.
        let external_symlink = vendor_directory.get_relative_str(EXTERNAL_ROOT_SYMLINK_NAME);
        fs_utils::ensure_symbolic_link(&external_symlink, external_repo_root.as_fragment())?;
        replant_symlinks(
            &repo_under_vendor,
            workspace,
            external_repo_root,
            &PathFragment::create(EXTERNAL_ROOT_SYMLINK_NAME),
            /* replant_symlinks_into_main_repo= */ true,
        )
        .map_err(|e| io::Error::other(e.get_message().to_string()))?;
        // 5. Rename the temporary marker file after the move/copy is done.
        temporary_marker.rename_to(&marker_under_vendor)?;
        // 6. Leave a symlink in external dir to keep things working.
        repo_under_external.delete_tree()?;
        fs_utils::ensure_symbolic_link(&repo_under_external, repo_under_vendor.as_fragment())?;
    }
    Ok(())
}

/// Checks if the repository under vendor dir is up-to-date by comparing its marker file with
/// the one under `<output_base>/external`. This function assumes the marker file under
/// `<output_base>/external` exists and is up-to-date.
// Java: com/google/devtools/build/lib/bazel/bzlmod/VendorManager.java:203 VendorManager.isRepoUpToDate
fn is_repo_up_to_date(
    marker_under_vendor: &Path,
    marker_under_external: &Path,
) -> io::Result<bool> {
    if !marker_under_vendor.exists() {
        return Ok(false);
    }
    // FileSystemUtils.readContent(..., UTF_8): both decode the same way, so the bytes compare as
    // the strings do.
    let vendor_marker_content =
        String::from_utf8_lossy(&fs_utils::read_content(marker_under_vendor)?).into_owned();
    let external_marker_content =
        String::from_utf8_lossy(&fs_utils::read_content(marker_under_external)?).into_owned();
    Ok(vendor_marker_content == external_marker_content)
}

/// Copies all dir trees under `from` to `to`; symbolic links are copied as links.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:575 FileSystemUtils.copyTreesBelow
pub fn copy_trees_below(from: &Path, to: &Path) -> io::Result<()> {
    if to.starts_with(from) {
        panic!(
            "IllegalArgumentException: {} is a subdirectory of {}",
            to, from
        );
    }
    for from_child in from.get_directory_entries()? {
        let to_child = to.get_child(from_child.get_base_name());
        if from_child.is_symbolic_link() {
            fs_utils::ensure_symbolic_link(&to_child, &from_child.read_symbolic_link()?)?;
        } else if from_child.is_directory_with(Symlinks::Nofollow) {
            to_child.create_directory()?;
            copy_trees_below(&from_child, &to_child)?;
        } else if from_child.is_file_with(Symlinks::Nofollow) {
            fs_utils::copy_file(&from_child, &to_child)?;
        } else {
            return Err(io::Error::other(format!(
                "Don't know how to copy {}",
                from_child
            )));
        }
    }
    Ok(())
}
