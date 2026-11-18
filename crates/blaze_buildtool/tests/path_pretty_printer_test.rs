//! Port of `src/test/java/com/google/devtools/build/lib/buildtool/PathPrettyPrinterTest.java`.

use blaze_buildtool::buildtool::path_pretty_printer::PathPrettyPrinter;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

fn symlinks(entries: &[(&str, &str)]) -> IndexMap<PathFragment, PathFragment> {
    entries
        .iter()
        .map(|(k, v)| (PathFragment::create(k), PathFragment::create(v)))
        .collect()
}

#[test]
fn get_pretty_path_path_under_symlink_target_returns_path_under_convenience_link() {
    let under_test = PathPrettyPrinter::new(
        /* workspaceRelativeWorkingDirectory= */ &PathFragment::empty_fragment(),
        /* symlinkPrefix= */ "ignored",
        symlinks(&[
            ("not-blaze-out", "/output/execroot/not-stuff"),
            ("blaze-out", "/output/execroot/stuff"),
        ]),
    );

    let path = PathFragment::create("/output/execroot/stuff/really");
    assert_eq!(
        under_test.get_pretty_path(&path),
        PathFragment::create("blaze-out/really")
    );
}

#[test]
fn get_pretty_path_working_directory_under_workspace_returns_up_level_reference() {
    let under_test = PathPrettyPrinter::new(
        &PathFragment::create("relative/working/directory"),
        /* symlinkPrefix= */ "ignored",
        symlinks(&[
            ("not-blaze-out", "/output/execroot/not-stuff"),
            ("blaze-out", "/output/execroot/stuff"),
        ]),
    );

    let path = PathFragment::create("/output/execroot/stuff/really");
    assert_eq!(
        under_test.get_pretty_path(&path),
        PathFragment::create("../../../blaze-out/really")
    );
}

#[test]
fn get_pretty_path_path_not_under_symlink_target_returns_original_path() {
    let under_test = PathPrettyPrinter::new(
        /* workspaceRelativeWorkingDirectory= */ &PathFragment::empty_fragment(),
        /* symlinkPrefix= */ "ignored",
        symlinks(&[("blaze-out", "/output/execroot/stuff")]),
    );

    let path = PathFragment::create("/output/execroot/not-stuff/really");
    assert_eq!(under_test.get_pretty_path(&path), path);
}

#[test]
fn get_pretty_path_no_create_symlinks_prefix_returns_original_path() {
    let under_test = PathPrettyPrinter::new(
        /* workspaceRelativeWorkingDirectory= */ &PathFragment::empty_fragment(),
        /* symlinkPrefix= */ "/",
        symlinks(&[("blaze-out", "/output/execroot/stuff")]),
    );

    let path = PathFragment::create("/output/execroot/stuff/really");
    assert_eq!(under_test.get_pretty_path(&path), path);
}
