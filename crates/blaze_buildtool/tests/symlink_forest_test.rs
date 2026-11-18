//! Port of `src/test/java/com/google/devtools/build/lib/buildtool/SymlinkForestTest.java`.
//!
//! PORT: Java runs on an `InMemoryFileSystem` rooted at `/`; these tests run on the real file
//! system under a temporary directory, so every Java absolute path `/x` is `<tmp>/x`.

use std::collections::HashSet;
use std::io::Write;

use blaze_buildtool::buildtool::symlink_forest::SymlinkForest;
use blaze_cmdline::label_constants::{
    EXTERNAL_PACKAGE_IDENTIFIER, EXTERNAL_PATH_PREFIX, EXTERNAL_REPOSITORY_LOCATION,
};
use blaze_cmdline::{PackageIdentifier, RepositoryName};
use blaze_vfs::{
    DigestHashFunction, FileSystemRef, Path, PathFragment, Root, Symlinks, UnixFileSystem,
};
use indexmap::{IndexMap, IndexSet};

// Java: src/test/java/com/google/devtools/build/lib/testutil/TestConstants.java:34 TestConstants.PRODUCT_NAME
const PRODUCT_NAME: &str = "bazel";

struct Fixture {
    _tmp: tempfile::TempDir,
    base: String,
    file_system: FileSystemRef,
    // The execution root.
    link_root: Path,
}

impl Fixture {
    // Java: SymlinkForestTest.initializeFileSystem
    fn new() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp
            .path()
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let file_system = UnixFileSystem::create(DigestHashFunction::Sha256);
        let link_root = Path::create_from_str(&format!("{}/linkRoot", base), &file_system);
        link_root.create_directory_and_parents().unwrap();
        Fixture {
            _tmp: tmp,
            base,
            file_system,
            link_root,
        }
    }

    /// `fileSystem.getPath(path)` for a Java absolute path.
    fn get_path(&self, path: &str) -> Path {
        Path::create_from_str(&format!("{}{}", self.base, path), &self.file_system)
    }
}

fn create_empty_file(path: &Path) {
    path.get_output_stream().unwrap().flush().unwrap();
}

struct TestTree {
    top_dir: Path,
    file1: Path,
    file2: Path,
    a_dir: Path,
}

/*
 * Build a directory tree that looks like:
 *   top-dir/
 *     file-1
 *     file-2
 *     a-dir/
 *       file-3
 *       inner-dir/
 *         link-1 => file-4
 *         dir-link => b-dir
 *   file-4
 */
fn create_test_directory_tree(f: &Fixture) -> TestTree {
    let top_dir = f.get_path("/top-dir");
    let file1 = f.get_path("/top-dir/file-1");
    let file2 = f.get_path("/top-dir/file-2");
    let a_dir = f.get_path("/top-dir/a-dir");
    let b_dir = f.get_path("/top-dir/b-dir");
    let file3 = f.get_path("/top-dir/a-dir/file-3");
    let inner_dir = f.get_path("/top-dir/a-dir/inner-dir");
    let link1 = f.get_path("/top-dir/a-dir/inner-dir/link-1");
    let dir_link = f.get_path("/top-dir/a-dir/inner-dir/dir-link");
    let file4 = f.get_path("/file-4");
    let file5 = f.get_path("/top-dir/b-dir/file-5");

    top_dir.create_directory().unwrap();
    create_empty_file(&file1);
    create_empty_file(&file2);
    a_dir.create_directory().unwrap();
    b_dir.create_directory().unwrap();
    create_empty_file(&file3);
    inner_dir.create_directory().unwrap();
    link1.create_symbolic_link(&file4).unwrap(); // simple symlink
    dir_link.create_symbolic_link(&b_dir).unwrap();
    create_empty_file(&file4);
    create_empty_file(&file5);
    TestTree {
        top_dir,
        file1,
        file2,
        a_dir,
    }
}

fn longest_path_prefix(path: &str, prefix_strs: &[&str]) -> Option<PathFragment> {
    let prefixes: IndexSet<PackageIdentifier> = prefix_strs
        .iter()
        .map(|p| PackageIdentifier::create_in_main_repo_str(p))
        .collect();
    let longest = SymlinkForest::longest_path_prefix(
        &PackageIdentifier::create_in_main_repo_str(path),
        |p| prefixes.contains(p),
    );
    longest.map(|l| l.get_package_fragment().clone())
}

fn external_source_root(output_base: &Root, repo_name: &str) -> Root {
    Root::from_path(
        output_base
            .as_path()
            .unwrap()
            .get_relative(&EXTERNAL_REPOSITORY_LOCATION)
            .get_relative_str(repo_name),
    )
}

#[test]
fn test_longest_path_prefix() {
    let a = PathFragment::create("A");
    assert_eq!(longest_path_prefix("A/b", &["A", "B"]), Some(a.clone())); // simple parent
    assert_eq!(longest_path_prefix("A", &["A", "B"]), Some(a.clone())); // self
    assert_eq!(
        longest_path_prefix("A/B/c", &["A", "A/B"]),
        Some(a.get_relative_str("B"))
    ); // want longest
    assert_eq!(longest_path_prefix("C/b", &["A", "B"]), None); // not found in other parents
    assert_eq!(longest_path_prefix("A", &["A/B", "B"]), None); // not found in child
    assert_eq!(
        longest_path_prefix("A/B/C/d/e/f.h", &["A/B/C", "B/C/d"]),
        Some(a.get_relative_str("B/C"))
    );
    assert_eq!(
        longest_path_prefix("A/f.h", &["", "B/C/d"]),
        Some(PathFragment::empty_fragment())
    );
}

#[test]
fn test_delete_trees_below_not_prefixed() {
    let f = Fixture::new();
    let t = create_test_directory_tree(&f);
    SymlinkForest::delete_trees_below_not_prefixed(&t.top_dir, "file-").unwrap();
    assert!(t.file1.exists());
    assert!(t.file2.exists());
    assert!(!t.a_dir.exists());
}

// Create same package under two different roots
fn create_pkg(root_a: Option<&Root>, root_b: Option<&Root>, pkg: &str) -> PackageIdentifier {
    if let Some(root_a) = root_a {
        root_a
            .get_relative_str(pkg)
            .create_directory_and_parents()
            .unwrap();
        create_empty_file(&root_a.get_relative_str(pkg).get_child("file"));
    }
    if let Some(root_b) = root_b {
        root_b
            .get_relative_str(pkg)
            .create_directory_and_parents()
            .unwrap();
        create_empty_file(&root_b.get_relative_str(pkg).get_child("file"));
    }
    PackageIdentifier::create_in_main_repo_str(pkg)
}

// Create package for external repo
fn create_external_pkg(root: Option<&Root>, repo: &str, pkg: &str) -> PackageIdentifier {
    if let Some(root) = root {
        let repo_root = root
            .get_relative(&EXTERNAL_REPOSITORY_LOCATION)
            .get_relative_str(repo);
        repo_root
            .get_relative_str(pkg)
            .create_directory_and_parents()
            .unwrap();
        create_empty_file(&repo_root.get_relative_str(pkg).get_child("file"));
    }
    PackageIdentifier::create(
        RepositoryName::create(repo).unwrap(),
        PathFragment::create(pkg),
    )
}

// Create package for main repo
fn create_main_pkg(repo_root: Option<&Root>, pkg: &str) -> PackageIdentifier {
    if let Some(repo_root) = repo_root {
        repo_root
            .get_relative_str(pkg)
            .create_directory_and_parents()
            .unwrap();
        create_empty_file(&repo_root.get_relative_str(pkg).get_child("file"));
    }
    PackageIdentifier::create_in_main_repo(PathFragment::create(pkg))
}

fn assert_links_to_root(from_root: &Path, to_root: &Root, relpart: &str) {
    assert_links_to(
        &from_root.get_relative_str(relpart),
        &to_root.get_relative_str(relpart),
    );
}

fn assert_links_to(from_root: &Path, to_root: &Path) {
    assert!(
        from_root.is_symbolic_link(),
        "stat: {:?}",
        from_root.stat().map(|s| s.is_directory())
    );
    assert_eq!(
        &from_root.read_symbolic_link().unwrap(),
        to_root.as_fragment()
    );
}

fn assert_links_to_external_repo(from_root: &Path, to_root: &Root, repo_name: &str) {
    assert_links_to(
        &from_root.get_relative(&EXTERNAL_PATH_PREFIX.get_relative_str(repo_name)),
        &to_root
            .get_relative(&EXTERNAL_REPOSITORY_LOCATION)
            .get_relative_str(repo_name),
    );
}

fn assert_is_dir(root: &Path, relpart: &str) {
    assert!(
        root.get_relative_str(relpart)
            .is_directory_with(Symlinks::Nofollow)
    );
}

/// Truth's `containsExactly` (order-insensitive).
fn assert_contains_exactly(actual: &[Path], expected: &[Path]) {
    let actual_set: HashSet<&Path> = actual.iter().collect();
    let expected_set: HashSet<&Path> = expected.iter().collect();
    assert_eq!(
        actual.len(),
        expected.len(),
        "{:?} vs {:?}",
        actual,
        expected
    );
    assert_eq!(actual_set, expected_set);
}

#[test]
fn test_plant_link_forest_with_multiple_package_path() {
    let f = Fixture::new();
    let root_a = Root::from_path(f.get_path("/A"));
    let root_b = Root::from_path(f.get_path("/B"));

    let mut package_root_map: IndexMap<PackageIdentifier, Root> = IndexMap::new();
    package_root_map.insert(
        create_pkg(Some(&root_a), Some(&root_b), "pkgA"),
        root_a.clone(),
    );
    package_root_map.insert(
        create_pkg(Some(&root_a), Some(&root_b), "dir1/pkgA"),
        root_a.clone(),
    );
    package_root_map.insert(
        create_pkg(Some(&root_a), Some(&root_b), "dir1/pkgB"),
        root_b.clone(),
    );
    package_root_map.insert(
        create_pkg(Some(&root_a), Some(&root_b), "dir2/pkg"),
        root_a.clone(),
    );
    package_root_map.insert(
        create_pkg(Some(&root_a), Some(&root_b), "dir2/pkg/pkg"),
        root_b.clone(),
    );
    package_root_map.insert(
        create_pkg(Some(&root_a), Some(&root_b), "pkgB"),
        root_b.clone(),
    );
    package_root_map.insert(
        create_pkg(Some(&root_a), Some(&root_b), "pkgB/dir/pkg"),
        root_a.clone(),
    );
    package_root_map.insert(
        create_pkg(Some(&root_a), Some(&root_b), "pkgB/pkg"),
        root_a.clone(),
    );
    package_root_map.insert(
        create_pkg(Some(&root_a), Some(&root_b), "pkgB/pkg/pkg"),
        root_a.clone(),
    );
    create_pkg(Some(&root_a), Some(&root_b), "pkgB/dir"); // create a file in there

    let link_root = f.get_path("/linkRoot");
    link_root.create_directory_and_parents().unwrap();
    let planted_symlinks = SymlinkForest::with_sibling_repository_layout(
        package_root_map,
        link_root.clone(),
        PRODUCT_NAME,
        false,
    )
    .plant_symlink_forest()
    .unwrap();

    assert_links_to_root(&link_root, &root_a, "pkgA");
    assert_is_dir(&link_root, "dir1");
    assert_links_to_root(&link_root, &root_a, "dir1/pkgA");
    assert_links_to_root(&link_root, &root_b, "dir1/pkgB");
    assert_is_dir(&link_root, "dir2");
    assert_is_dir(&link_root, "dir2/pkg");
    assert_links_to_root(&link_root, &root_a, "dir2/pkg/file");
    assert_links_to_root(&link_root, &root_b, "dir2/pkg/pkg");
    assert_is_dir(&link_root, "pkgB");
    assert_is_dir(&link_root, "pkgB/dir");
    assert_links_to_root(&link_root, &root_b, "pkgB/dir/file");
    assert_links_to_root(&link_root, &root_a, "pkgB/dir/pkg");
    assert_links_to_root(&link_root, &root_a, "pkgB/pkg");
    assert_contains_exactly(
        &planted_symlinks,
        &[
            link_root.get_relative_str("pkgA"),
            link_root.get_relative_str("dir1/pkgA"),
            link_root.get_relative_str("dir1/pkgB"),
            link_root.get_relative_str("dir2/pkg/file"),
            link_root.get_relative_str("dir2/pkg/pkg"),
            link_root.get_relative_str("pkgB/file"),
            link_root.get_relative_str("pkgB/dir/file"),
            link_root.get_relative_str("pkgB/dir/pkg"),
            link_root.get_relative_str("pkgB/pkg"),
        ],
    );
}

#[test]
fn test_top_level_package() {
    let f = Fixture::new();
    let root_x = Root::from_path(f.get_path("/X"));
    let root_y = Root::from_path(f.get_path("/Y"));
    let mut package_root_map: IndexMap<PackageIdentifier, Root> = IndexMap::new();
    package_root_map.insert(create_pkg(Some(&root_x), Some(&root_y), ""), root_x.clone());
    package_root_map.insert(
        create_pkg(Some(&root_x), Some(&root_y), "foo"),
        root_x.clone(),
    );

    let planted_symlinks = SymlinkForest::with_sibling_repository_layout(
        package_root_map,
        f.link_root.clone(),
        PRODUCT_NAME,
        false,
    )
    .plant_symlink_forest()
    .unwrap();
    assert_links_to_root(&f.link_root, &root_x, "file");
    assert_contains_exactly(
        &planted_symlinks,
        &[
            f.link_root.get_relative_str("file"),
            f.link_root.get_relative_str("foo"),
        ],
    );
}

/// The package root map shared by `testPlantSymlinkForest` and
/// `test_withSiblingRepoLayout_plantSymlinkForest`.
fn main_and_external_packages(
    output_base: &Root,
    main_repo: &Root,
) -> IndexMap<PackageIdentifier, Root> {
    let mut m: IndexMap<PackageIdentifier, Root> = IndexMap::new();
    m.insert(
        create_main_pkg(Some(main_repo), "dir_main"),
        main_repo.clone(),
    );
    m.insert(
        create_main_pkg(Some(main_repo), "dir_lib/pkg"),
        main_repo.clone(),
    );
    m.insert(create_main_pkg(Some(main_repo), ""), main_repo.clone());
    // Remote repo without top-level package.
    m.insert(
        create_external_pkg(Some(output_base), "X", "dir_x/pkg"),
        external_source_root(output_base, "X"),
    );
    // Remote repo with and without top-level package.
    m.insert(
        create_external_pkg(Some(output_base), "Y", ""),
        external_source_root(output_base, "Y"),
    );
    m.insert(
        create_external_pkg(Some(output_base), "Y", "dir_y/pkg"),
        external_source_root(output_base, "Y"),
    );
    // Only top-level pkg.
    m.insert(
        create_external_pkg(Some(output_base), "Z", ""),
        external_source_root(output_base, "Z"),
    );
    m
}

#[test]
fn test_plant_symlink_forest() {
    let f = Fixture::new();
    let output_base = Root::from_path(f.get_path("/ob"));
    let main_repo = Root::from_path(f.get_path("/my_repo"));
    let link_root = output_base.get_relative_str("execroot/ws_name");

    main_repo
        .as_path()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    link_root.create_directory_and_parents().unwrap();

    let package_root_map = main_and_external_packages(&output_base, &main_repo);

    let planted_symlinks = SymlinkForest::with_sibling_repository_layout(
        package_root_map,
        link_root.clone(),
        PRODUCT_NAME,
        false,
    )
    .plant_symlink_forest()
    .unwrap();

    assert_links_to_root(&link_root, &main_repo, "dir_main");
    assert_links_to_root(&link_root, &main_repo, "dir_lib");
    assert_links_to_root(&link_root, &main_repo, "file");
    assert_links_to_external_repo(&link_root, &output_base, "X");
    assert_links_to_external_repo(&link_root, &output_base, "Y");
    assert_links_to_external_repo(&link_root, &output_base, "Z");
    assert!(
        link_root
            .get_relative(&EXTERNAL_PATH_PREFIX)
            .get_relative_str("Y/file")
            .exists()
    );
    assert!(
        link_root
            .get_relative(&EXTERNAL_PATH_PREFIX)
            .get_relative_str("Z/file")
            .exists()
    );
    assert_contains_exactly(
        &planted_symlinks,
        &[
            link_root.get_relative_str("dir_main"),
            link_root.get_relative_str("dir_lib"),
            link_root.get_relative_str("file"),
            link_root.get_relative_str(&format!("{}/X", *EXTERNAL_PATH_PREFIX)),
            link_root.get_relative_str(&format!("{}/Y", *EXTERNAL_PATH_PREFIX)),
            link_root.get_relative_str(&format!("{}/Z", *EXTERNAL_PATH_PREFIX)),
        ],
    );
}

#[test]
fn test_with_sibling_repo_layout_plant_symlink_forest() {
    let f = Fixture::new();
    let output_base = Root::from_path(f.get_path("/ob"));
    let main_repo = Root::from_path(f.get_path("/my_repo"));
    let link_root = output_base.get_relative_str("execroot/ws_name");

    main_repo
        .as_path()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    link_root.create_directory_and_parents().unwrap();

    let package_root_map = main_and_external_packages(&output_base, &main_repo);

    let planted_symlinks = SymlinkForest::with_sibling_repository_layout(
        package_root_map,
        link_root.clone(),
        PRODUCT_NAME,
        true,
    )
    .plant_symlink_forest()
    .unwrap();

    // Expected sibling repository layout (X, Y and Z are siblings of ws_name):
    //
    // .
    // ├── execroot
    // │   ├── ws_name { ... }
    // │   ├── X -> external/X
    // │   ├── Y -> external/Y
    // │   └── Z -> external/Z
    // └── external
    //     ├── X
    //     ├── Y
    //     └── Z

    let parent = link_root.get_parent_directory().unwrap();
    assert_links_to_root(&link_root, &main_repo, "dir_main");
    assert_links_to_root(&link_root, &main_repo, "dir_lib");
    assert_links_to_root(&link_root, &main_repo, "file");
    for repo in ["X", "Y", "Z"] {
        assert_links_to(
            &parent.get_relative_str(repo),
            &output_base.get_relative_str(&format!("{}/{}", *EXTERNAL_REPOSITORY_LOCATION, repo)),
        );
    }
    assert!(parent.get_relative_str("Y/file").exists());
    assert!(parent.get_relative_str("Z/file").exists());
    assert_contains_exactly(
        &planted_symlinks,
        &[
            link_root.get_relative_str("dir_main"),
            link_root.get_relative_str("dir_lib"),
            link_root.get_relative_str("file"),
            parent.get_relative_str("X"),
            parent.get_relative_str("Y"),
            parent.get_relative_str("Z"),
        ],
    );
}

#[test]
fn test_plant_symlink_forest_for_main_repo() {
    // For the main repo, plantSymlinkForest function should only link all files and dirs under
    // main repo root that're presented in packageRootMap.
    let f = Fixture::new();
    let output_base = Root::from_path(f.get_path("/ob"));
    let main_repo = Root::from_path(f.get_path("/my_repo"));
    let link_root = output_base.get_relative_str("execroot/ws_name");

    link_root.create_directory_and_parents().unwrap();
    main_repo
        .as_path()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    main_repo
        .get_relative_str("dir4")
        .create_directory_and_parents()
        .unwrap();
    create_empty_file(&main_repo.get_relative_str("file"));

    let mut package_root_map: IndexMap<PackageIdentifier, Root> = IndexMap::new();
    package_root_map.insert(
        create_main_pkg(Some(&main_repo), "dir1/pkg/foo"),
        main_repo.clone(),
    );
    package_root_map.insert(
        create_main_pkg(Some(&main_repo), "dir2/pkg"),
        main_repo.clone(),
    );
    package_root_map.insert(create_main_pkg(Some(&main_repo), "dir3"), main_repo.clone());
    package_root_map.insert(
        create_external_pkg(Some(&output_base), "X", "dir_x/pkg"),
        external_source_root(&output_base, "X"),
    );

    let planted_symlinks = SymlinkForest::with_sibling_repository_layout(
        package_root_map,
        link_root.clone(),
        PRODUCT_NAME,
        false,
    )
    .plant_symlink_forest()
    .unwrap();

    assert_links_to_root(&link_root, &main_repo, "dir1");
    assert_links_to_root(&link_root, &main_repo, "dir2");
    assert_links_to_root(&link_root, &main_repo, "dir3");
    // dir4 and the file under main repo root should not be linked
    // because they are not presented in packageRootMap.
    assert!(!link_root.get_child("dir4").exists());
    assert!(!link_root.get_child("file").exists());
    assert_links_to_external_repo(&link_root, &output_base, "X");
    assert_contains_exactly(
        &planted_symlinks,
        &[
            link_root.get_relative_str("dir1"),
            link_root.get_relative_str("dir2"),
            link_root.get_relative_str("dir3"),
            link_root.get_relative_str(&format!("{}/X", *EXTERNAL_PATH_PREFIX)),
        ],
    );
}

#[test]
fn test_with_subdir_repo_layout_test_external_dir_in_main_repo_is_ignored1() {
    // Test external/ is ignored even when packages like "//external/foo" is specified.
    let f = Fixture::new();
    let output_base = Root::from_path(f.get_path("/ob"));
    let main_repo = Root::from_path(f.get_path("/my_repo"));
    let link_root = output_base.get_relative_str("execroot/ws_name");

    link_root.create_directory_and_parents().unwrap();
    main_repo
        .as_path()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();

    let mut package_root_map: IndexMap<PackageIdentifier, Root> = IndexMap::new();
    package_root_map.insert(
        create_main_pkg(Some(&main_repo), "dir1/pkg/foo"),
        main_repo.clone(),
    );
    package_root_map.insert(
        create_main_pkg(Some(&main_repo), "dir2/pkg"),
        main_repo.clone(),
    );
    package_root_map.insert(create_main_pkg(Some(&main_repo), "dir3"), main_repo.clone());
    // external/ should not be linked even we have "//external/foo" package
    package_root_map.insert(
        create_main_pkg(Some(&main_repo), "external/foo"),
        main_repo.clone(),
    );
    package_root_map.insert(
        create_external_pkg(Some(&output_base), "X", "dir_x/pkg"),
        external_source_root(&output_base, "X"),
    );

    let planted_symlinks = SymlinkForest::with_sibling_repository_layout(
        package_root_map,
        link_root.clone(),
        PRODUCT_NAME,
        false,
    )
    .plant_symlink_forest()
    .unwrap();

    assert_links_to_root(&link_root, &main_repo, "dir1");
    assert_links_to_root(&link_root, &main_repo, "dir2");
    assert_links_to_root(&link_root, &main_repo, "dir3");
    assert_links_to_external_repo(&link_root, &output_base, "X");
    assert!(!output_base.get_relative_str("external/foo").exists());
    assert_contains_exactly(
        &planted_symlinks,
        &[
            link_root.get_relative_str("dir1"),
            link_root.get_relative_str("dir2"),
            link_root.get_relative_str("dir3"),
            link_root.get_relative_str(&format!("{}/X", *EXTERNAL_PATH_PREFIX)),
        ],
    );
}

#[test]
fn test_with_sub_dir_repo_layout_test_external_dir_in_main_repo_is_ignored2() {
    // Test external/ is ignored when root package "//:" is specified.
    let f = Fixture::new();
    let output_base = Root::from_path(f.get_path("/ob"));
    let main_repo = Root::from_path(f.get_path("/my_repo"));
    let link_root = output_base.get_relative_str("execroot/ws_name");

    link_root.create_directory_and_parents().unwrap();
    main_repo
        .as_path()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    main_repo
        .get_relative_str("dir3")
        .create_directory_and_parents()
        .unwrap();
    main_repo
        .get_relative_str("external/foo")
        .create_directory_and_parents()
        .unwrap();

    let mut package_root_map: IndexMap<PackageIdentifier, Root> = IndexMap::new();
    package_root_map.insert(
        create_main_pkg(Some(&main_repo), "dir1/pkg/foo"),
        main_repo.clone(),
    );
    package_root_map.insert(
        create_main_pkg(Some(&main_repo), "dir2/pkg"),
        main_repo.clone(),
    );
    // Empty package will cause every top-level files to be linked, except external/
    package_root_map.insert(create_main_pkg(Some(&main_repo), ""), main_repo.clone());
    package_root_map.insert(
        create_external_pkg(Some(&output_base), "X", "dir_x/pkg"),
        external_source_root(&output_base, "X"),
    );

    let planted_symlinks = SymlinkForest::with_sibling_repository_layout(
        package_root_map,
        link_root.clone(),
        PRODUCT_NAME,
        false,
    )
    .plant_symlink_forest()
    .unwrap();

    assert_links_to_root(&link_root, &main_repo, "dir1");
    assert_links_to_root(&link_root, &main_repo, "dir2");
    assert_links_to_root(&link_root, &main_repo, "dir3");
    assert_links_to_external_repo(&link_root, &output_base, "X");
    assert!(!output_base.get_relative_str("external/foo").exists());
    assert_contains_exactly(
        &planted_symlinks,
        &[
            link_root.get_relative_str("dir1"),
            link_root.get_relative_str("dir2"),
            link_root.get_relative_str("dir3"),
            link_root.get_relative_str("file"),
            link_root.get_relative_str(&format!("{}/X", *EXTERNAL_PATH_PREFIX)),
        ],
    );
}

#[test]
fn test_with_sibling_repo_layout_test_external_dir_in_main_repo_exists() {
    // Test external/ is ignored even when packages like "//external/foo" is specified.
    let f = Fixture::new();
    let output_base = Root::from_path(f.get_path("/ob"));
    let main_repo = Root::from_path(f.get_path("/my_repo"));
    let link_root = output_base.get_relative_str("execroot/ws_name");

    link_root.create_directory_and_parents().unwrap();
    main_repo
        .as_path()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();

    let mut package_root_map: IndexMap<PackageIdentifier, Root> = IndexMap::new();
    package_root_map.insert(
        create_main_pkg(Some(&main_repo), "dir1/pkg/foo"),
        main_repo.clone(),
    );
    package_root_map.insert(
        create_main_pkg(Some(&main_repo), "dir2/pkg"),
        main_repo.clone(),
    );
    package_root_map.insert(create_main_pkg(Some(&main_repo), "dir3"), main_repo.clone());
    // external/ should not be linked even we have "//external/foo" package
    package_root_map.insert(
        create_main_pkg(Some(&main_repo), "external/foo"),
        main_repo.clone(),
    );
    package_root_map.insert(
        create_external_pkg(Some(&output_base), "X", "dir_x/pkg"),
        external_source_root(&output_base, "X"),
    );

    let planted_symlinks = SymlinkForest::with_sibling_repository_layout(
        package_root_map,
        link_root.clone(),
        PRODUCT_NAME,
        true,
    )
    .plant_symlink_forest()
    .unwrap();

    // Expected output base layout with sibling repositories in the execroot where
    // ws_name and X are siblings:
    //
    // /ob
    // ├── execroot
    // │   ├── ws_name
    // │   │   ├── dir1
    // │   │   │   └── pkg
    // │   │   │       └── foo -> /my_repo/dir1/pkg/foo
    // │   │   ├── dir2
    // │   │   │   └── pkg -> /my_repo/dir2/pkg
    // │   │   ├── dir3 -> /my_repo/dir3
    // │   │   └── external -> /my_repo/external
    // │   └── X -> /ob/external/X
    // └── external
    //     └── X

    assert_links_to_root(&link_root, &main_repo, "dir1");
    assert_links_to_root(&link_root, &main_repo, "dir2");
    assert_links_to_root(&link_root, &main_repo, "dir3");

    assert!(
        output_base
            .get_relative(&EXTERNAL_REPOSITORY_LOCATION)
            .get_relative_str("X")
            .exists()
    );
    assert!(output_base.get_relative_str("execroot/X").exists());
    assert_links_to(
        &link_root
            .get_parent_directory()
            .unwrap()
            .get_relative_str("X"), // Sibling of the main repo.
        &output_base
            .get_relative(&EXTERNAL_REPOSITORY_LOCATION)
            .get_relative_str("X"),
    );

    assert!(link_root.get_relative_str("external/foo").exists());

    assert_contains_exactly(
        &planted_symlinks,
        &[
            link_root.get_relative_str("dir1"),
            link_root.get_relative_str("dir2"),
            link_root.get_relative_str("dir3"),
            link_root.get_relative_str("external"), // Symlinked to the main repo's top level external dir
            link_root
                .get_parent_directory()
                .unwrap()
                .get_relative_str("X"), // Symlinked to /ob/external/X
        ],
    );
}

#[test]
fn test_with_sibling_repo_layout_and_root_package_in_roots_test_external_dir_in_main_repo_exists() {
    // Test external/ is ignored when root package "//:" is specified.
    let f = Fixture::new();
    let output_base = Root::from_path(f.get_path("/ob"));
    let main_repo = Root::from_path(f.get_path("/my_repo"));
    let link_root = output_base.get_relative_str("execroot/ws_name");

    link_root.create_directory_and_parents().unwrap();
    main_repo
        .as_path()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();

    main_repo
        .get_relative_str("external/foo")
        .create_directory_and_parents()
        .unwrap();

    let mut package_root_map: IndexMap<PackageIdentifier, Root> = IndexMap::new();
    package_root_map.insert(create_main_pkg(Some(&main_repo), ""), main_repo.clone());
    package_root_map.insert(
        create_external_pkg(Some(&output_base), "X", "dir_x/pkg"),
        external_source_root(&output_base, "X"),
    );

    let planted_symlinks = SymlinkForest::with_sibling_repository_layout(
        package_root_map,
        link_root.clone(),
        PRODUCT_NAME,
        true,
    )
    .plant_symlink_forest()
    .unwrap();

    // Expected output base layout with sibling repositories in the execroot where
    // ws_name and X are siblings:
    //
    // /ob
    // ├── execroot
    // │   ├── ws_name
    // │   │   └── external -> /my_repo/external
    // │   └── X -> /ob/external/X
    // └── external
    //     └── X

    assert!(
        output_base
            .get_relative(&EXTERNAL_REPOSITORY_LOCATION)
            .get_relative_str("X")
            .exists()
    );
    assert!(output_base.get_relative_str("execroot/X").exists());
    assert_links_to(
        &link_root
            .get_parent_directory()
            .unwrap()
            .get_relative_str("X"), // Sibling of the main repo.
        &output_base
            .get_relative(&EXTERNAL_REPOSITORY_LOCATION)
            .get_relative_str("X"),
    );

    assert!(link_root.get_relative_str("external/foo").exists());

    assert_contains_exactly(
        &planted_symlinks,
        &[
            link_root
                .get_parent_directory()
                .unwrap()
                .get_relative_str("X"),
            link_root.get_relative_str("file"), // created by createMainPkg test setup
            link_root.get_relative_str("external"), // symlink to main repo's top level external directory
        ],
    );
}

#[test]
fn test_external_package() {
    let f = Fixture::new();
    let link_root = f.get_path("/linkRoot");
    link_root.create_directory_and_parents().unwrap();

    let root = Root::from_path(f.get_path("/src"));
    let mut package_root_map: IndexMap<PackageIdentifier, Root> = IndexMap::new();
    // Virtual root, shouldn't actually be linked in.
    package_root_map.insert(EXTERNAL_PACKAGE_IDENTIFIER.clone(), root);

    let planted_symlinks = SymlinkForest::with_sibling_repository_layout(
        package_root_map,
        link_root.clone(),
        PRODUCT_NAME,
        false,
    )
    .plant_symlink_forest()
    .unwrap();
    assert!(!link_root.get_relative(&EXTERNAL_PATH_PREFIX).exists());
    assert!(planted_symlinks.is_empty());
}
