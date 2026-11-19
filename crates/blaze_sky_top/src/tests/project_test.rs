//! Port of `src/test/java/com/google/devtools/build/lib/analysis/ProjectTest.java`.
//!
//! PORT: Java extends `AnalysisTestCase`; the harness's `BuildViewTestCase` provides the same
//! scratch workspace and Skyframe executor.

use blaze_cmdline::Label;
use indexmap::IndexSet;

use crate::project::find_project_files;
use crate::testing::BuildViewTestCase;

const PROJECT_FILE_NAME: &str = "PROJECT.scl";

// Java: ProjectTest.defineSimpleRule
fn set_up() -> BuildViewTestCase {
    let t = BuildViewTestCase::new();
    t.scratch.file(
        "foo/defs.bzl",
        &[
            "simple_rule = rule(",
            "    implementation = lambda ctx: [],",
            "    attrs = {},",
            ")",
        ],
    );
    t
}

fn simple_rule(t: &BuildViewTestCase, build_file: &str, name: &str) {
    t.scratch.file(
        build_file,
        &[
            "load(\"//foo:defs.bzl\", \"simple_rule\")",
            &format!("simple_rule(name = \"{name}\")"),
        ],
    );
}

fn label(s: &str) -> Label {
    Label::parse_canonical(s).unwrap()
}

/// `Project.findProjectFiles(targets, skyframeExecutor, reporter).asMap()`.
fn find(
    t: &mut BuildViewTestCase,
    targets: &[&str],
) -> Result<Vec<(Label, Vec<Label>)>, crate::project::ProjectResolutionException> {
    let targets: IndexSet<Label> = targets.iter().map(|l| label(l)).collect();
    let handler = t.event_handler();
    find_project_files(&targets, t.skyframe_executor(), handler)
}

/// `containsExactly(k1, v1, k2, v2, ...)` on a map: the same entries, in any order.
fn assert_contains_exactly(actual: Vec<(Label, Vec<Label>)>, expected: &[(&str, &[&str])]) {
    let expected: Vec<(Label, Vec<Label>)> = expected
        .iter()
        .map(|(k, v)| (label(k), v.iter().map(|l| label(l)).collect()))
        .collect();
    assert_eq!(actual.len(), expected.len(), "{actual:?}");
    for entry in &expected {
        assert!(actual.contains(entry), "{actual:?} lacks {entry:?}");
    }
}

// Java: ProjectTest.singleTargetNoProjects
#[test]
fn single_target_no_projects() {
    let mut t = set_up();
    simple_rule(&t, "foo/bar/BUILD", "s");
    assert!(find(&mut t, &["//foo/bar:s"]).unwrap().is_empty());
}

// Java: ProjectTest.singleTargetProjectInDirectPackage
#[test]
fn single_target_project_in_direct_package() {
    let mut t = set_up();
    simple_rule(&t, "foo/bar/BUILD", "s");
    t.scratch.file(&format!("foo/bar/{PROJECT_FILE_NAME}"), &[]);
    assert_contains_exactly(
        find(&mut t, &["//foo/bar:s"]).unwrap(),
        &[("//foo/bar:s", &["//foo/bar:PROJECT.scl"])],
    );
}

// Java: ProjectTest.singleTargetProjectInParentPackage
#[test]
fn single_target_project_in_parent_package() {
    let mut t = set_up();
    simple_rule(&t, "foo/bar/BUILD", "s");
    t.scratch.file("foo/BUILD", &[]);
    t.scratch.file(&format!("foo/{PROJECT_FILE_NAME}"), &[]);
    assert_contains_exactly(
        find(&mut t, &["//foo/bar:s"]).unwrap(),
        &[("//foo/bar:s", &["//foo:PROJECT.scl"])],
    );
}

// Java: ProjectTest.singleTargetProjectInBothDirectAndParentPackages
#[test]
fn single_target_project_in_both_direct_and_parent_packages() {
    let mut t = set_up();
    simple_rule(&t, "foo/bar/BUILD", "s");
    t.scratch.file("foo/BUILD", &[]);
    t.scratch.file(&format!("foo/{PROJECT_FILE_NAME}"), &[]);
    t.scratch.file(&format!("foo/bar/{PROJECT_FILE_NAME}"), &[]);
    assert_contains_exactly(
        find(&mut t, &["//foo/bar:s"]).unwrap(),
        &[(
            "//foo/bar:s",
            &["//foo/bar:PROJECT.scl", "//foo:PROJECT.scl"],
        )],
    );
}

// Java: ProjectTest.singleTargetProjectInNonPackageParentDir
#[test]
fn single_target_project_in_non_package_parent_dir() {
    let mut t = set_up();
    simple_rule(&t, "foo/bar/BUILD", "s");
    t.scratch.file(&format!("foo/{PROJECT_FILE_NAME}"), &[]);
    t.scratch.file(&format!("foo/bar/{PROJECT_FILE_NAME}"), &[]);
    // Project files don't count if they're in directories without BUILD files.
    assert_contains_exactly(
        find(&mut t, &["//foo/bar:s"]).unwrap(),
        &[("//foo/bar:s", &["//foo/bar:PROJECT.scl"])],
    );
}

// Java: ProjectTest.twoTargetsInIndependentPackages
#[test]
fn two_targets_in_independent_packages() {
    let mut t = set_up();
    simple_rule(&t, "foo/BUILD", "s");
    simple_rule(&t, "baz/BUILD", "t");
    t.scratch.file(&format!("foo/{PROJECT_FILE_NAME}"), &[]);
    t.scratch.file(&format!("baz/{PROJECT_FILE_NAME}"), &[]);
    assert_contains_exactly(
        find(&mut t, &["//foo:s", "//baz:t"]).unwrap(),
        &[
            ("//foo:s", &["//foo:PROJECT.scl"]),
            ("//baz:t", &["//baz:PROJECT.scl"]),
        ],
    );
}

// Java: ProjectTest.twoTargetsInSubPackagesHierarchy
#[test]
fn two_targets_in_sub_packages_hierarchy() {
    let mut t = set_up();
    simple_rule(&t, "foo/bar/BUILD", "child");
    simple_rule(&t, "foo/BUILD", "parent");
    t.scratch.file(&format!("foo/bar/{PROJECT_FILE_NAME}"), &[]);
    t.scratch.file(&format!("foo/{PROJECT_FILE_NAME}"), &[]);
    assert_contains_exactly(
        find(&mut t, &["//foo:parent", "//foo/bar:child"]).unwrap(),
        &[
            ("//foo:parent", &["//foo:PROJECT.scl"]),
            (
                "//foo/bar:child",
                &["//foo/bar:PROJECT.scl", "//foo:PROJECT.scl"],
            ),
        ],
    );
}

// Java: ProjectTest.testInvalidProjectFile
#[test]
fn test_invalid_project_file() {
    let mut t = set_up();
    simple_rule(&t, "foo/BUILD", "myrule");
    t.scratch.dir(&format!("foo/{PROJECT_FILE_NAME}"));
    assert!(find(&mut t, &["//foo:myrule"]).is_err());
}
