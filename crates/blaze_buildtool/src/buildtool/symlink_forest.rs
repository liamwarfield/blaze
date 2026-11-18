//! Port of `buildtool/SymlinkForest.java`: plants `<output_base>/execroot/<main repo>` with
//! symlinks into the source roots and the external repositories.

use std::collections::HashMap;
use std::io;

use blaze_cmdline::label_constants::{EXTERNAL_PACKAGE_IDENTIFIER, EXTERNAL_PATH_PREFIX};
use blaze_cmdline::{PackageIdentifier, RepositoryName};
use blaze_vfs::{Path, PathFragment, Root};
use indexmap::{IndexMap, IndexSet};

/// `IgnoredSubdirectories.matchingEntry` of the ignored subdirectories passed to
/// [`SymlinkForest::eagerly_plant_symlink_forest_single_package_path`] and
/// [`SymlinkForest::symlink_should_be_planted_ignoring`].
///
/// PORT: `IgnoredSubdirectories` is ported in `blaze_sky_loading` (pending its move to
/// `blaze_cmdline`), which this crate does not depend on; callers pass its `matching_entry`.
pub type IgnoredSubdirectoriesMatcher<'a> = &'a dyn Fn(&PathFragment) -> Option<String>;

/// `IgnoredSubdirectories.EMPTY.matchingEntry`: matches nothing.
// Java: com/google/devtools/build/lib/cmdline/IgnoredSubdirectories.java:39 IgnoredSubdirectories.EMPTY
fn empty_ignored_subdirectories(_directory: &PathFragment) -> Option<String> {
    None
}

/// Creates a symlink forest based on a package path map.
// Java: com/google/devtools/build/lib/buildtool/SymlinkForest.java:49 SymlinkForest
pub struct SymlinkForest {
    package_roots: IndexMap<PackageIdentifier, Root>,
    execroot: Path,
    product_name: String,
    prefix: String,
    sibling_repository_layout: bool,
}

impl SymlinkForest {
    /// Constructor for a symlink forest creator without non-symlinked directories parameter.
    // Java: com/google/devtools/build/lib/buildtool/SymlinkForest.java:59 SymlinkForest.<init>
    pub fn new(
        package_roots: IndexMap<PackageIdentifier, Root>,
        execroot: Path,
        product_name: &str,
    ) -> SymlinkForest {
        Self::with_sibling_repository_layout(package_roots, execroot, product_name, false)
    }

    /// Constructor for a symlink forest creator; does not perform any i/o.
    ///
    /// Use [`SymlinkForest::plant_symlink_forest`] to actually create the symlink forest.
    ///
    /// `package_roots` are the source package roots to which to create symlinks, `execroot` the
    /// path where to plant the symlink forest, `product_name` is `BlazeRuntime#getProductName()`.
    // Java: com/google/devtools/build/lib/buildtool/SymlinkForest.java:73 SymlinkForest.<init>
    pub fn with_sibling_repository_layout(
        package_roots: IndexMap<PackageIdentifier, Root>,
        execroot: Path,
        product_name: &str,
        sibling_repository_layout: bool,
    ) -> SymlinkForest {
        SymlinkForest {
            package_roots,
            execroot,
            product_name: product_name.to_string(),
            prefix: format!("{}-", product_name),
            sibling_repository_layout,
        }
    }

    /// Returns the longest prefix from a given set of 'prefixes' that are contained in 'path'.
    /// I.e the closest ancestor directory containing path. Returns `None` if none found.
    // Java: com/google/devtools/build/lib/buildtool/SymlinkForest.java:94 SymlinkForest.longestPathPrefix
    ///
    /// PORT: `Set<PackageIdentifier> prefixes` is passed as its `contains`.
    pub fn longest_path_prefix(
        path: &PackageIdentifier,
        prefixes: impl Fn(&PackageIdentifier) -> bool,
    ) -> Option<PackageIdentifier> {
        let mut i = path.get_package_fragment().segment_count() as i64;
        while i >= 0 {
            let prefix = create_in_repo(
                path,
                path.get_package_fragment().sub_fragment(0, i as usize),
            );
            if prefixes(&prefix) {
                return Some(prefix);
            }
            i -= 1;
        }
        None
    }

    /// Delete all dir trees under a given 'dir' that don't start with a given 'prefix', and is not
    /// special case of not symlinked to exec root directories (those directories are special case
    /// of output roots, so they must be kept before commands). Does not follow any symbolic links.
    // Java: com/google/devtools/build/lib/buildtool/SymlinkForest.java:112 SymlinkForest.deleteTreesBelowNotPrefixed
    pub fn delete_trees_below_not_prefixed(dir: &Path, prefix: &str) -> io::Result<()> {
        for p in dir.get_directory_entries()? {
            if p.get_base_name().starts_with(prefix) {
                continue;
            }

            p.delete_tree()?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/buildtool/SymlinkForest.java:122 SymlinkForest.plantSymlinkForExternalRepo
    fn plant_symlink_for_external_repo(
        &self,
        planted_symlinks: &mut Vec<Path>,
        repository: &RepositoryName,
        source: &Path,
        external_repo_links: &mut IndexSet<Path>,
    ) -> io::Result<()> {
        let planted_symlink = Self::plant_single_symlink_for_external_repo(
            repository,
            source,
            &self.execroot,
            self.sibling_repository_layout,
            external_repo_links,
        )?;
        if let Some(planted_symlink) = planted_symlink {
            planted_symlinks.push(planted_symlink);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/buildtool/SymlinkForest.java:134 SymlinkForest.plantSymlinkForestWithFullMainRepository
    fn plant_symlink_forest_with_full_main_repository(
        &self,
        planted_symlinks: &mut Vec<Path>,
        main_repo_root: &Path,
    ) -> io::Result<()> {
        // For the main repo top-level directory, generate symlinks to everything in the directory
        // instead of the directory itself.
        if self.sibling_repository_layout {
            self.execroot.create_directory()?;
        }
        for target in main_repo_root.get_directory_entries()? {
            let base_name = target.get_base_name();
            let exec_path = self.execroot.get_relative_str(base_name);
            if Self::symlink_should_be_planted(
                &self.prefix,
                self.sibling_repository_layout,
                base_name,
                &target,
            ) {
                exec_path.create_symbolic_link(&target)?;
                planted_symlinks.push(exec_path);
                // TODO(jingwen-external): is this creating execroot/io_bazel/external?
            }
        }
        Ok(())
    }

    /// PORT: Java declares `AbruptExitException` but never throws it.
    // Java: com/google/devtools/build/lib/buildtool/SymlinkForest.java:152 SymlinkForest.plantSymlinkForestWithPartialMainRepository
    fn plant_symlink_forest_with_partial_main_repository(
        &self,
        planted_symlinks: &mut Vec<Path>,
        main_repo_links: &IndexMap<Path, Path>,
    ) -> io::Result<()> {
        if self.sibling_repository_layout {
            self.execroot.create_directory()?;
        }
        for (link, target) in main_repo_links {
            link.create_symbolic_link(target)?;
            planted_symlinks.push(link.clone());
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/buildtool/SymlinkForest.java:166 SymlinkForest.plantSymlinkForestMultiPackagePath
    fn plant_symlink_forest_multi_package_path(
        &self,
        planted_symlinks: &mut Vec<Path>,
        package_roots_for_main_repo: &IndexMap<PackageIdentifier, Root>,
    ) -> io::Result<()> {
        let sibling = self.sibling_repository_layout;
        let execroot = &self.execroot;
        let is_main_pkg = |p: &PackageIdentifier| package_roots_for_main_repo.contains_key(p);
        // Packages come from exactly one root, but their shared ancestors may come from more.
        // PORT: Java's HashMap/HashSet; their iteration order is never observed.
        let mut dir_roots_map: HashMap<PackageIdentifier, IndexSet<Root>> = HashMap::new();
        // Elements in this list are added so that parents come before their children.
        let mut dirs_parents_first: Vec<PackageIdentifier> = Vec::new();
        for (pkg_id, pkg_root) in package_roots_for_main_repo {
            let mut new_dirs: Vec<PackageIdentifier> = Vec::new();
            let mut fragment = pkg_id.get_package_fragment().clone();
            while !fragment.is_empty() {
                let dir_id = create_in_repo(pkg_id, fragment.clone());
                let roots = dir_roots_map.entry(dir_id.clone()).or_insert_with(|| {
                    new_dirs.push(dir_id.clone());
                    IndexSet::new()
                });
                roots.insert(pkg_root.clone());
                fragment = fragment
                    .get_parent_directory()
                    .unwrap_or_else(PathFragment::empty_fragment);
            }
            new_dirs.reverse();
            dirs_parents_first.extend(new_dirs);
        }
        // Now add in roots for all non-pkg dirs that are in between two packages, and missed
        // above.
        for dir in &dirs_parents_first {
            if !package_roots_for_main_repo.contains_key(dir)
                && let Some(pkg_id) = Self::longest_path_prefix(dir, is_main_pkg)
            {
                let root = package_roots_for_main_repo.get(&pkg_id).unwrap().clone();
                dir_roots_map.get_mut(dir).unwrap().insert(root);
            }
        }
        // Create output dirs for all dirs that have more than one root and need to be split.
        for dir in &dirs_parents_first {
            if !dir.get_repository().is_main() {
                execroot
                    .get_relative(&dir.get_repository().get_exec_path(sibling))
                    .create_directory_and_parents()?;
            }
            if dir_roots_map.get(dir).unwrap().len() > 1 {
                // PORT: skipped, logging (`logger.atFiner().log("mkdir %s", ...)`).
                execroot
                    .get_relative(&dir.get_exec_path(sibling))
                    .create_directory_and_parents()?;
            }
        }

        // Make dir links for single rooted dirs.
        for dir in &dirs_parents_first {
            let roots = dir_roots_map.get(dir).unwrap();
            // Simple case of one root for this dir.
            if roots.len() == 1 {
                let parent = dir
                    .get_package_fragment()
                    .get_parent_directory()
                    .unwrap_or_else(PathFragment::empty_fragment);
                if !parent.is_empty()
                    && dir_roots_map
                        .get(&create_in_repo(dir, parent))
                        .unwrap()
                        .len()
                        == 1
                {
                    continue; // skip--an ancestor will link this one in from above
                }
                // This is the top-most dir that can be linked to a single root. Make it so.
                let root = roots.get_index(0).unwrap(); // lone root in set
                let link = execroot.get_relative(&dir.get_exec_path(sibling));
                // PORT: skipped, logging (`logger.atFiner().log("ln -s %s %s", ...)`).
                link.create_symbolic_link(&root.get_relative(dir.get_source_root()))?;
                planted_symlinks.push(link);
            }
        }
        // Make links for dirs within packages, skip parent-only dirs.
        for dir in &dirs_parents_first {
            if dir_roots_map.get(dir).unwrap().len() > 1 {
                // If this dir is at or below a package dir, link in its contents.
                if let Some(pkg_id) = Self::longest_path_prefix(dir, is_main_pkg) {
                    let root = package_roots_for_main_repo.get(&pkg_id).unwrap();
                    let result: io::Result<()> = (|| {
                        let absdir = root.get_relative(dir.get_source_root());
                        if absdir.is_directory() {
                            // PORT: skipped, logging (`logger.atFiner().log("ln -s %s/* %s/")`).
                            for target in absdir.get_directory_entries()? {
                                let p = root.relativize(&target);
                                if !dir_roots_map.contains_key(&create_in_repo(&pkg_id, p.clone()))
                                {
                                    execroot.get_relative(&p).create_symbolic_link(&target)?;
                                    planted_symlinks.push(execroot.get_relative(&p));
                                }
                            }
                        } else {
                            // PORT: skipped, logging (`logger.atFine().log("Symlink planting
                            // skipping dir '%s'", absdir)`).
                        }
                        Ok(())
                    })();
                    if let Err(_e) = result {
                        // TODO(arostovtsev): Why are we swallowing the IOException here instead
                        // of letting it be thrown?
                        // PORT: skipped, logging (`logger.atWarning().withCause(e).log("I/O
                        // error while planting symlinks to contents of '%s'", ...)`).
                    }
                    // Otherwise its just an otherwise empty common parent dir.
                }
            }
        }

        for (pkg_id, pkg_root) in package_roots_for_main_repo {
            if !pkg_id.get_package_fragment().is_empty() {
                continue;
            }
            let execroot_directory = execroot.get_relative(&pkg_id.get_exec_path(sibling));
            // If there were no subpackages, this directory might not exist yet.
            if !execroot_directory.exists() {
                execroot_directory.create_directory_and_parents()?;
            }
            // For the top-level directory, generate symlinks to everything in the directory
            // instead of the directory itself.
            let source_directory = pkg_root.get_relative(pkg_id.get_source_root());
            for target in source_directory.get_directory_entries()? {
                let base_name = target.get_base_name();
                let exec_path = execroot_directory.get_relative_str(base_name);
                // Create any links that don't exist yet and don't start with bazel-.
                if !base_name.starts_with(&format!("{}-", self.product_name)) && !exec_path.exists()
                {
                    exec_path.create_symbolic_link(&target)?;
                    planted_symlinks.push(exec_path);
                }
            }
        }
        Ok(())
    }

    /// Performs the filesystem operations to plant the symlink forest.
    ///
    /// Returns the symlinks that have been planted.
    ///
    /// PORT: Java also declares `AbruptExitException`, which nothing below throws.
    // Java: com/google/devtools/build/lib/buildtool/SymlinkForest.java:297 SymlinkForest.plantSymlinkForest
    pub fn plant_symlink_forest(&self) -> io::Result<Vec<Path>> {
        Self::delete_trees_below_not_prefixed(&self.execroot, &self.prefix)?;
        Self::delete_sibling_repository_symlinks(self.sibling_repository_layout, &self.execroot)?;

        let mut should_link_all_top_level_items = false;
        let mut main_repo_links: IndexMap<Path, Path> = IndexMap::new();
        let mut main_repo_roots: IndexSet<Root> = IndexSet::new();
        let mut external_repo_links: IndexSet<Path> = IndexSet::new();
        let mut package_roots_for_main_repo: IndexMap<PackageIdentifier, Root> = IndexMap::new();
        let mut planted_symlinks: Vec<Path> = Vec::new();

        for (pkg_id, root) in &self.package_roots {
            if *pkg_id == *EXTERNAL_PACKAGE_IDENTIFIER {
                // //external is a virtual package regardless , don't add it to the symlink tree.
                // Subpackages of //external, like //external/foo, are fine though.
                continue;
            }
            let repository = pkg_id.get_repository();
            if repository.is_main() {
                // Record main repo packages.
                package_roots_for_main_repo.insert(pkg_id.clone(), root.clone());

                // Record the root of the packages.
                main_repo_roots.insert(root.clone());

                // For single root (single package path) case:
                // If root package of the main repo is required, we record the main repo root so
                // that we can later link everything under the main repo's top-level directory.
                // If root package of the main repo is not required, we only record links for
                // directories under the top-level directory that are used in required packages.
                if pkg_id.get_package_fragment().is_empty() {
                    should_link_all_top_level_items = true;
                } else {
                    let base_name = pkg_id.get_package_fragment().get_segment(0);
                    if !self.sibling_repository_layout
                        && base_name == EXTERNAL_PATH_PREFIX.get_base_name()
                    {
                        // ignore external/ directory if user has it in the source tree
                        // because it conflicts with external repository location.
                        continue;
                    }
                    let execroot_link = self.execroot.get_relative_str(base_name);
                    let source_path = root.get_relative_str(pkg_id.get_top_level_dir());
                    main_repo_links.entry(execroot_link).or_insert(source_path);
                }
            } else {
                let source = root
                    .as_path()
                    .expect("Root.asPath() of an external repository root")
                    .clone();
                self.plant_symlink_for_external_repo(
                    &mut planted_symlinks,
                    repository,
                    &source,
                    &mut external_repo_links,
                )?;
            }
        }

        // TODO(bazel-team): Bazel can find packages in multiple paths by specifying
        // --package_paths, we need a more complex algorithm to build execroot in that case. As
        // --package_path will be removed in the future, we should remove the
        // plantSymlinkForestMultiPackagePath implementation when --package_path is gone.
        if main_repo_roots.len() > 1 {
            self.plant_symlink_forest_multi_package_path(
                &mut planted_symlinks,
                &package_roots_for_main_repo,
            )?;
        } else if should_link_all_top_level_items {
            blaze_util::check_argument!(
                main_repo_roots.len() == 1,
                "expected one element but was: {:?}",
                main_repo_roots
            );
            let main_repo_root = main_repo_roots
                .get_index(0)
                .unwrap()
                .as_path()
                .expect("Root.asPath() of the main repository root")
                .clone();
            self.plant_symlink_forest_with_full_main_repository(
                &mut planted_symlinks,
                &main_repo_root,
            )?;
        } else {
            self.plant_symlink_forest_with_partial_main_repository(
                &mut planted_symlinks,
                &main_repo_links,
            )?;
        }

        // PORT: skipped, logging (`logger.atInfo().log("Planted symlink forest in %s")`).
        Ok(planted_symlinks)
    }

    // Java: com/google/devtools/build/lib/buildtool/SymlinkForest.java:365 SymlinkForest.deleteSiblingRepositorySymlinks
    fn delete_sibling_repository_symlinks(
        sibling_repository_layout: bool,
        execroot: &Path,
    ) -> io::Result<()> {
        if sibling_repository_layout {
            // Delete execroot/../<symlinks> to directories representing external repositories.
            let parent = execroot
                .get_parent_directory()
                .expect("execroot has a parent directory");
            for p in parent.get_directory_entries()? {
                if p.is_symbolic_link() {
                    p.delete_tree()?;
                }
            }
        }
        Ok(())
    }

    /// Eagerly plant the symlinks from execroot to the source root provided by the single package
    /// path of the current build. Only works with a single package path. Before planting the new
    /// symlinks, remove all existing symlinks in execroot which don't match certain criteria.
    ///
    /// It's possible to have a conflict here. For example when we plant symlinks form a
    /// case-insensitive FS to a case-sensitive one.
    ///
    /// Returns a set of potentially conflicting baseNames, all in lowercase.
    ///
    /// PORT: Java groups the entries in a `HashMap` and returns `ImmutableSet.copyOf(HashSet)`.
    /// The planting order is not observable; the returned names are in directory-listing order.
    // Java: com/google/devtools/build/lib/buildtool/SymlinkForest.java:387 SymlinkForest.eagerlyPlantSymlinkForestSinglePackagePath
    pub fn eagerly_plant_symlink_forest_single_package_path(
        execroot: &Path,
        source_root: &Path,
        prefix: &str,
        ignored_paths: IgnoredSubdirectoriesMatcher<'_>,
        sibling_repository_layout: bool,
    ) -> io::Result<IndexSet<String>> {
        Self::delete_trees_below_not_prefixed(execroot, prefix)?;
        Self::delete_sibling_repository_symlinks(sibling_repository_layout, execroot)?;

        let mut symlink_base_name_to_targets: IndexMap<String, Vec<Path>> = IndexMap::new();
        let mut potentially_conflicting_base_names_lowercase: IndexSet<String> = IndexSet::new();
        for target in source_root.get_directory_entries()? {
            let base_name_lowercase = target.get_base_name().to_ascii_lowercase();
            symlink_base_name_to_targets
                .entry(base_name_lowercase)
                .or_default()
                .push(target);
        }

        for (base_name_lowercase, targets) in &symlink_base_name_to_targets {
            // Easy case: there's no clashing expected. Just plant with the ORIGINAL base name.
            if targets.len() == 1 {
                let target = &targets[0];
                let original_base_name = target.get_base_name();
                let link = execroot.get_relative_str(original_base_name);
                if Self::symlink_should_be_planted_ignoring(
                    prefix,
                    ignored_paths,
                    sibling_repository_layout,
                    original_base_name,
                    target,
                ) {
                    link.create_symbolic_link(target)?;
                }
            } else {
                potentially_conflicting_base_names_lowercase.insert(base_name_lowercase.clone());
            }
        }
        Ok(potentially_conflicting_base_names_lowercase)
    }

    // Java: com/google/devtools/build/lib/buildtool/SymlinkForest.java:425 SymlinkForest.symlinkShouldBePlanted
    pub fn symlink_should_be_planted(
        prefix: &str,
        sibling_repository_layout: bool,
        base_name: &str,
        target: &Path,
    ) -> bool {
        Self::symlink_should_be_planted_ignoring(
            prefix,
            &empty_ignored_subdirectories,
            sibling_repository_layout,
            base_name,
            target,
        )
    }

    // Java: com/google/devtools/build/lib/buildtool/SymlinkForest.java:431 SymlinkForest.symlinkShouldBePlanted
    pub fn symlink_should_be_planted_ignoring(
        prefix: &str,
        ignored_subdirectories: IgnoredSubdirectoriesMatcher<'_>,
        sibling_repository_layout: bool,
        base_name: &str,
        target: &Path,
    ) -> bool {
        // Create any links that don't start with bazel-, and ignore external/ directory if
        // user has it in the source tree because it conflicts with external repository location.
        !base_name.starts_with(prefix)
            && ignored_subdirectories(&target.as_fragment().to_relative()).is_none()
            && (sibling_repository_layout || base_name != EXTERNAL_PATH_PREFIX.get_base_name())
    }

    /// Performs the planting of a symlink to an external repository.
    ///
    /// Returns the planted symlink, or `None` if nothing was planted.
    // Java: com/google/devtools/build/lib/buildtool/SymlinkForest.java:451 SymlinkForest.plantSingleSymlinkForExternalRepo
    pub fn plant_single_symlink_for_external_repo(
        repository: &RepositoryName,
        source: &Path,
        execroot: &Path,
        sibling_repository_layout: bool,
        already_planted_external_repo_links: &mut IndexSet<Path>,
    ) -> io::Result<Option<Path>> {
        // For external repositories, create one symlink to each external repository
        // directory.
        // From <output_base>/execroot/<main repo name>/external/<external repo name>
        // to   <output_base>/external/<external repo name>
        //
        // However, if --experimental_sibling_repository_layout is true, symlink:
        // From <output_base>/execroot/<external repo name>
        // to   <output_base>/external/<external repo name>
        let execroot_link =
            execroot.get_relative(&repository.get_exec_path(sibling_repository_layout));

        if !sibling_repository_layout && already_planted_external_repo_links.is_empty() {
            execroot
                .get_relative(&EXTERNAL_PATH_PREFIX)
                .create_directory_and_parents()?;
        }
        // Prevent re-creating existing symlinks.
        if !already_planted_external_repo_links.insert(execroot_link.clone()) {
            return Ok(None);
        }
        execroot_link.create_symbolic_link(source)?;
        Ok(Some(execroot_link))
    }
}

// Java: com/google/devtools/build/lib/buildtool/SymlinkForest.java:479 SymlinkForest.createInRepo
fn create_in_repo(repo: &PackageIdentifier, package_fragment: PathFragment) -> PackageIdentifier {
    PackageIdentifier::create(repo.get_repository().clone(), package_fragment)
}

/// Checked exception for issues with Symlink planting.
// Java: com/google/devtools/build/lib/buildtool/SymlinkForest.java:485 SymlinkForest.SymlinkPlantingException
#[derive(Debug, thiserror::Error)]
#[error("{msg}")]
pub struct SymlinkPlantingException {
    pub msg: String,
    #[source]
    pub cause: io::Error,
}

impl SymlinkPlantingException {
    // Java: com/google/devtools/build/lib/buildtool/SymlinkForest.java:486 SymlinkPlantingException.<init>
    pub fn new(msg: impl Into<String>, e: io::Error) -> SymlinkPlantingException {
        SymlinkPlantingException {
            msg: msg.into(),
            cause: e,
        }
    }
}
