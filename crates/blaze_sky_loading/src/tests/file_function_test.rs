//! Port of FileFunctionTest (the parts that do not need incremental re-evaluation or Java's
//! in-memory file system with injected failures).
//!
//! Java: src/test/java/com/google/devtools/build/lib/skyframe/FileFunctionTest.java. Java runs on
//! an in-memory file system rooted at `/root`; Blaze runs on a scratch directory of the real file
//! system (`pkgRoot` = the scratch workspace).

use std::sync::Arc;

use blaze_events::StoredEventHandler;
use blaze_skyframe::{Key, SkyKey};
use blaze_vfs::{FileSystemExt, Path, PathFragment, Root, RootedPath};

use crate::actions::FileValue;
use crate::file::FileKey;
use crate::file_state::FileStateKey;
use crate::io::{IOException, IOExceptionKind};
use crate::test_util::{FakeRepos, TestSetup, evaluate};

struct Fixture {
    setup: TestSetup,
    pkg_root: Root,
    output_base_root: Root,
}

impl Fixture {
    // Java: FileFunctionTest.createFsAndRoot
    fn new() -> Fixture {
        let setup = TestSetup::new(FakeRepos::default());
        let pkg_root = Root::from_path(setup.workspace().clone());
        let output_base_root = Root::from_path(setup.config.directories.get_output_base().clone());
        Fixture {
            setup,
            pkg_root,
            output_base_root,
        }
    }

    fn path(&self, root_relative_path: &str) -> Path {
        self.pkg_root
            .get_relative(&PathFragment::create(root_relative_path))
    }

    fn file(&self, file_name: &str) -> Path {
        self.file_with(file_name, "")
    }

    fn file_with(&self, file_name: &str, contents: &str) -> Path {
        let p = self.path(file_name);
        self.setup.scratch.file(p.get_path_string(), contents)
    }

    fn directory(&self, directory_name: &str) -> Path {
        self.setup
            .scratch
            .dir(self.path(directory_name).get_path_string())
    }

    fn symlink(&self, link: &str, target: &str) -> Path {
        self.setup
            .scratch
            .symlink(self.path(link).get_path_string(), target)
    }

    fn rooted_path(&self, path_string: &str) -> RootedPath {
        let roots = [self.pkg_root.clone(), self.output_base_root.clone()];
        RootedPath::to_rooted_path_maybe_under_root(&self.path(path_string), roots.iter())
    }

    fn sky_key(&self, path_string: &str) -> FileKey {
        FileKey::key(self.rooted_path(path_string))
    }

    fn value_for_rooted_path(&self, rooted_path: RootedPath) -> FileValue {
        let evaluator = self.setup.evaluator();
        let key = FileKey::key(rooted_path);
        let result = evaluate(
            &evaluator,
            vec![SkyKey::new(key.clone())],
            false,
            Arc::new(StoredEventHandler::new()),
        );
        assert!(!result.has_error(), "{:?}", result);
        (*result.get(&key).unwrap()).clone()
    }

    fn value_for_path(&self, path: &Path) -> FileValue {
        self.value_for_rooted_path(RootedPath::to_rooted_path(
            self.pkg_root.clone(),
            &self.pkg_root.relativize(path),
        ))
    }

    fn value_for_path_outside_pkg_root(&self, path: &Path) -> FileValue {
        let root = self.setup.scratch.fs.absolute_root();
        self.value_for_rooted_path(RootedPath::to_rooted_path(
            root.clone(),
            &root.relativize(path),
        ))
    }

    // Java: FileFunctionTest.assertNoError
    fn assert_no_error(&self, path_string: &str) {
        let evaluator = self.setup.evaluator();
        let key = self.sky_key(path_string);
        let result = evaluate(
            &evaluator,
            vec![SkyKey::new(key.clone())],
            false,
            Arc::new(StoredEventHandler::new()),
        );
        assert!(
            !result.has_error(),
            "Did not expect error while evaluating {}, got {:?}",
            path_string,
            result
        );
    }

    // Java: FileFunctionTest.assertError
    fn assert_error(&self, path_string: &str) {
        let evaluator = self.setup.evaluator();
        let key = self.sky_key(path_string);
        let result = evaluate(
            &evaluator,
            vec![SkyKey::new(key.clone())],
            false,
            Arc::new(StoredEventHandler::new()),
        );
        assert!(
            result.has_error(),
            "Expected error while evaluating {}, got {:?}",
            path_string,
            result
        );
        let error = result.get_first_error().unwrap();
        assert!(!error.get_cycle_info().is_empty() || error.get_exception().is_some());
    }

    // Java: FileFunctionTest.checkRealPath
    fn check_real_path(&self, path_string: &str) {
        let real_path = self.path(path_string).resolve_symbolic_links().unwrap();
        self.assert_real_path(path_string, self.pkg_root.relativize(&real_path).as_ref());
    }

    // Java: FileFunctionTest.assertRealPath
    fn assert_real_path(&self, path_string: &str, expected_real_path_string: &str) {
        let evaluator = self.setup.evaluator();
        let key = self.sky_key(path_string);
        let result = evaluate(
            &evaluator,
            vec![SkyKey::new(key.clone())],
            false,
            Arc::new(StoredEventHandler::new()),
        );
        if result.has_error() {
            panic!("Evaluation error for {:?}: {:?}", key, result);
        }
        let file_value = result.get(&key).unwrap();
        assert_eq!(
            file_value.real_rooted_path(&key.0).as_path().to_string(),
            self.pkg_root
                .get_relative(&PathFragment::create(expected_real_path_string))
                .to_string()
        );
    }
}

#[test]
fn test_file_value_hash_code_and_equals_contract() {
    let f = Fixture::new();
    let path_a = f.file_with("a", "a");
    let path_b = f.file_with("b", "b");
    let path_c = f.symlink("c", "a");
    let path_d = f.directory("d");
    let path_da = f.file_with("d/a", "da");
    let path_e = f.symlink("e", "d");
    let path_f = f.symlink("f", "a");

    let v = |p: &Path| f.value_for_path_outside_pkg_root(p);
    // PORT: Java's symlink targets are root-relative ("a"); the values compare the real paths
    // and link targets, which match here too.
    super::equals_tester(&[
        vec![v(&path_a), v(&path_a)],
        vec![v(&path_b), v(&path_b)],
        // Both 'f' and 'c' are transitively symlinks to 'a', so all of these FileValues ought to
        // be equal.
        vec![v(&path_c), v(&path_c), v(&path_f), v(&path_f)],
        vec![v(&path_d), v(&path_d)],
        vec![v(&path_da), v(&path_da)],
        vec![v(&path_e), v(&path_e)],
    ]);
}

#[test]
fn test_is_directory() {
    let f = Fixture::new();
    assert!(!f.value_for_path(&f.file("a")).is_directory());
    assert!(!f.value_for_path(&f.path("nonexistent")).is_directory());
    assert!(f.value_for_path(&f.directory("dir")).is_directory());

    assert!(!f.value_for_path(&f.symlink("sa", "a")).is_directory());
    assert!(
        !f.value_for_path(&f.symlink("smissing", "missing"))
            .is_directory()
    );
    assert!(f.value_for_path(&f.symlink("sdir", "dir")).is_directory());
    assert!(f.value_for_path(&f.symlink("ssdir", "sdir")).is_directory());
}

#[test]
fn test_is_file() {
    let f = Fixture::new();
    assert!(f.value_for_path(&f.file("a")).is_file());
    assert!(!f.value_for_path(&f.path("nonexistent")).is_file());
    assert!(!f.value_for_path(&f.directory("dir")).is_file());

    assert!(f.value_for_path(&f.symlink("sa", "a")).is_file());
    assert!(
        !f.value_for_path(&f.symlink("smissing", "missing"))
            .is_file()
    );
    assert!(!f.value_for_path(&f.symlink("sdir", "dir")).is_file());
    assert!(f.value_for_path(&f.symlink("ssfile", "sa")).is_file());
}

#[test]
fn test_self_referencing_symlink() {
    let f = Fixture::new();
    f.symlink("a", "a");
    f.assert_error("a");
}

#[test]
fn test_mutually_referencing_symlinks() {
    let f = Fixture::new();
    f.symlink("a", "b");
    f.symlink("b", "a");
    f.assert_error("a");
}

#[test]
fn test_recursive_nesting_symlink() {
    let f = Fixture::new();
    f.symlink("a/a", "../a");
    f.file("b");
    f.assert_no_error("a/a/a/a/b");
}

#[test]
fn test_simple_unbounded_ancestor_symlink_expansion_chain_reported() {
    let f = Fixture::new();
    f.symlink("a/a", "../a");
    let v = f.value_for_path(&f.path("a/a"));
    let chain = v.unbounded_ancestor_symlink_expansion_chain().unwrap();
    // Java: containsExactly (any order).
    assert_eq!(chain.len(), 2);
    assert!(chain.contains(&f.rooted_path("a/a")));
    assert!(chain.contains(&f.rooted_path("a")));
}

#[test]
fn test_traverse_into_virtual_non_directory() {
    let f = Fixture::new();
    f.file("dir/a");
    f.symlink("vdir", "dir");
    // The following evaluation should not throw IOExceptions.
    f.assert_no_error("vdir/a/aa/aaa");
}

#[test]
fn test_file_creation() {
    let f = Fixture::new();
    let a = f.value_for_path(&f.path("file"));
    let p = f.file("file");
    let b = f.value_for_path(&p);
    assert_ne!(a, b);
}

#[test]
fn test_real_path() {
    let f = Fixture::new();
    f.file("file");
    f.directory("directory");
    f.file("directory/file");
    f.symlink("directory/link", "file");
    f.symlink("directory/doublelink", "link");
    f.symlink("directory/parentlink", "../file");
    f.symlink("directory/doubleparentlink", "../link");
    f.symlink("link", "file");
    f.symlink("deadlink", "missing_file");
    f.symlink("dirlink", "directory");
    f.symlink("doublelink", "link");
    f.symlink("doubledirlink", "dirlink");

    f.check_real_path("file");
    f.check_real_path("link");
    f.check_real_path("doublelink");

    for dir in ["directory", "dirlink", "doubledirlink"] {
        f.check_real_path(dir);
        f.check_real_path(&format!("{}/file", dir));
        f.check_real_path(&format!("{}/link", dir));
        f.check_real_path(&format!("{}/doublelink", dir));
        f.check_real_path(&format!("{}/parentlink", dir));
    }

    f.assert_real_path("missing", "missing");
    f.assert_real_path("deadlink", "missing_file");
}

#[test]
fn test_real_path_relative_symlink() {
    let f = Fixture::new();
    f.directory("dir");
    f.symlink("dir/link", "../dir2");
    f.directory("dir2");
    f.symlink("dir2/filelink", "../dest");
    f.file("dest");

    f.check_real_path("dir/link/filelink");
}

// Java: FileFunctionTest.runTestSymlinkCycle
fn run_test_symlink_cycle(ancestor_cycle: bool, start_in_cycle: bool) {
    let f = Fixture::new();
    f.symlink("a", "b");
    f.symlink("b", "c");
    f.symlink("c", "d");
    f.symlink("d", "e");
    f.symlink("e", "c");
    let rp = |s: &str| f.rooted_path(s);
    // We build multiple keys at once to make sure the cycle is reported exactly once.
    let start_to_cycle = |start: &RootedPath| -> Vec<RootedPath> {
        let s = start.get_root_relative_path().get_path_string().to_owned();
        if s.starts_with('d') {
            vec![rp("d"), rp("e"), rp("c")]
        } else if s.starts_with('e') {
            vec![rp("e"), rp("c"), rp("d")]
        } else {
            vec![rp("c"), rp("d"), rp("e")]
        }
    };
    let start_to_path_to_cycle = |start: &RootedPath| -> Vec<RootedPath> {
        let s = start.get_root_relative_path().get_path_string().to_owned();
        if s.starts_with('a') {
            vec![rp("a"), rp("b")]
        } else if s.starts_with('b') {
            vec![rp("b")]
        } else {
            vec![]
        }
    };
    let keys: Vec<FileKey> = if ancestor_cycle && start_in_cycle {
        vec![
            f.sky_key("d/some/descendant"),
            f.sky_key("e/some/descendant"),
        ]
    } else if ancestor_cycle {
        vec![
            f.sky_key("a/some/descendant"),
            f.sky_key("b/some/descendant"),
        ]
    } else if start_in_cycle {
        vec![f.sky_key("d"), f.sky_key("e")]
    } else {
        vec![f.sky_key("a"), f.sky_key("b")]
    };
    let event_handler = Arc::new(StoredEventHandler::new());
    let evaluator = f.setup.evaluator();
    let result = evaluate(
        &evaluator,
        keys.iter().cloned().map(SkyKey::new).collect(),
        true,
        event_handler.clone(),
    );
    assert!(result.has_error());
    for key in &keys {
        let error_info = result.get_error(&SkyKey::new(key.clone())).unwrap();
        // FileFunction detects symlink cycles explicitly.
        assert!(error_info.get_cycle_info().is_empty());
        let fsce = error_info.exception_as::<IOException>().unwrap();
        let IOExceptionKind::FileSymlinkCycle {
            path_to_cycle,
            cycle,
        } = fsce.kind()
        else {
            panic!("not a FileSymlinkCycleException: {:?}", fsce);
        };
        let start = &key.0;
        assert_eq!(*path_to_cycle, start_to_path_to_cycle(start));
        assert_eq!(*cycle, start_to_cycle(start));
    }
    // Check that the unique cycle was reported exactly once.
    let events = event_handler.get_events();
    assert_eq!(events.len(), 1);
    assert!(
        events[0]
            .get_message()
            .contains("circular symlinks detected")
    );
}

#[test]
fn test_symlink_cycle_ancestor_cycle_start_in_cycle() {
    run_test_symlink_cycle(true, true);
}

#[test]
fn test_symlink_cycle_ancestor_cycle_start_out_of_cycle() {
    run_test_symlink_cycle(true, false);
}

#[test]
fn test_symlink_cycle_regular_cycle_start_in_cycle() {
    run_test_symlink_cycle(false, true);
}

#[test]
fn test_symlink_cycle_regular_cycle_start_out_of_cycle() {
    run_test_symlink_cycle(false, false);
}

#[test]
fn test_file_state_equality() {
    let f = Fixture::new();
    f.file("a");
    f.symlink("b1", "a");
    f.symlink("b2", "a");
    f.symlink("b3", "zzz");
    f.directory("d1");
    f.directory("d2");
    let k = |s: &str| FileStateKey::key(f.rooted_path(s));
    let names = ["a", "b1", "b2", "b3", "c1", "c2", "d1", "d2"];
    let keys: Vec<FileStateKey> = names.iter().map(|n| k(n)).collect();
    let evaluator = f.setup.evaluator();
    let result = evaluate(
        &evaluator,
        keys.iter().cloned().map(SkyKey::new).collect(),
        false,
        Arc::new(StoredEventHandler::new()),
    );
    let v = |i: usize| (*result.get(&keys[i]).unwrap()).clone();
    super::equals_tester(&[
        vec![v(0)],
        vec![v(1), v(2)],
        vec![v(3)],
        vec![v(4), v(5)],
        vec![v(6), v(7)],
    ]);
}

#[test]
fn test_symlink_to_package_path_boundary() {
    let f = Fixture::new();
    let path = f.path("this/is/a/path");
    f.setup.scratch.symlink(
        path.get_path_string(),
        f.pkg_root.as_path().unwrap().get_path_string(),
    );
    f.assert_no_error("this/is/a/path");
}

// Java: FileFunctionTest.runTestSimpleInfiniteSymlinkExpansion
fn run_test_simple_infinite_symlink_expansion(symlink_to_ancestor: bool, absolute_symlink: bool) {
    let f = Fixture::new();
    let other_path = f.path("other");
    let other_rooted_path = f.rooted_path("other");
    let ancestor_path = f.path("a");
    let ancestor_rooted_path = f.rooted_path("a");
    f.setup.scratch.symlink(
        other_path.get_path_string(),
        ancestor_path.get_path_string(),
    );
    let intermediate_path = f.path("inter");
    let intermediate_rooted_path = f.rooted_path("inter");
    let descendant_path = f.path("a/b/c/d/e");
    let descendant_rooted_path = f.rooted_path("a/b/c/d/e");
    if symlink_to_ancestor {
        f.setup.scratch.symlink(
            descendant_path.get_path_string(),
            intermediate_path.get_path_string(),
        );
        if absolute_symlink {
            f.setup.scratch.symlink(
                intermediate_path.get_path_string(),
                ancestor_path.get_path_string(),
            );
        } else {
            f.setup.scratch.symlink(
                intermediate_path.get_path_string(),
                ancestor_rooted_path
                    .get_root_relative_path()
                    .get_path_string(),
            );
        }
    } else {
        f.setup.scratch.symlink(
            ancestor_path.get_path_string(),
            intermediate_path.get_path_string(),
        );
        if absolute_symlink {
            f.setup.scratch.symlink(
                intermediate_path.get_path_string(),
                descendant_path.get_path_string(),
            );
        } else {
            f.setup.scratch.symlink(
                intermediate_path.get_path_string(),
                descendant_rooted_path
                    .get_root_relative_path()
                    .get_path_string(),
            );
        }
    }
    let event_handler = Arc::new(StoredEventHandler::new());
    let evaluator = f.setup.evaluator();
    let ancestor_path_key = FileKey::key(ancestor_rooted_path.clone());
    let descendant_path_key = FileKey::key(descendant_rooted_path.clone());
    let other_path_key = FileKey::key(other_rooted_path);
    let (keys, error_keys, expected_chain) = if symlink_to_ancestor {
        (
            vec![descendant_path_key.clone(), other_path_key],
            vec![descendant_path_key],
            vec![
                descendant_rooted_path,
                intermediate_rooted_path,
                ancestor_rooted_path,
            ],
        )
    } else {
        let keys = vec![ancestor_path_key, other_path_key];
        (
            keys.clone(),
            keys,
            vec![
                ancestor_rooted_path,
                intermediate_rooted_path,
                descendant_rooted_path,
            ],
        )
    };

    let result = evaluate(
        &evaluator,
        keys.iter().cloned().map(SkyKey::new).collect(),
        true,
        event_handler.clone(),
    );
    if symlink_to_ancestor {
        assert!(!result.has_error(), "{:?}", result);
    } else {
        assert!(result.has_error());
        for key in &error_keys {
            let error_info = result.get_error(&SkyKey::new(key.clone())).unwrap();
            // FileFunction detects infinite symlink expansion explicitly.
            assert!(error_info.get_cycle_info().is_empty());
            let fsiee = error_info.exception_as::<IOException>().unwrap();
            assert!(fsiee.get_message().contains("Infinite symlink expansion"));
            let IOExceptionKind::FileSymlinkInfiniteExpansion { chain, .. } = fsiee.kind() else {
                panic!("not a FileSymlinkInfiniteExpansionException: {:?}", fsiee);
            };
            assert_eq!(*chain, expected_chain);
        }
        // Check that the unique symlink expansion error was reported exactly once.
        let events = event_handler.get_events();
        assert_eq!(events.len(), 1);
        assert!(
            events[0]
                .get_message()
                .contains("infinite symlink expansion detected")
        );
    }
}

#[test]
fn test_infinite_symlink_expansion_absolute_symlink_to_descendant() {
    run_test_simple_infinite_symlink_expansion(false, true);
}

#[test]
fn test_infinite_symlink_expansion_relative_symlink_to_descendant() {
    run_test_simple_infinite_symlink_expansion(false, false);
}

#[test]
fn test_infinite_symlink_expansion_absolute_symlink_to_ancestor() {
    run_test_simple_infinite_symlink_expansion(true, true);
}

#[test]
fn test_infinite_symlink_expansion_relative_symlink_to_ancestor() {
    run_test_simple_infinite_symlink_expansion(true, false);
}

#[test]
fn test_infinite_symlink_expansion_symlink_to_referrer_to_ancestor() {
    let f = Fixture::new();
    f.symlink("d", "a");
    f.directory("a/b");
    f.symlink("a/b/c", "../../d/b");
    f.symlink("e", "a/b/c");
    let f_path = f.symlink("f", "e");

    let rooted_path_f =
        RootedPath::to_rooted_path(f.pkg_root.clone(), &f.pkg_root.relativize(&f_path));
    let key_f = FileKey::key(rooted_path_f);

    let evaluator = f.setup.evaluator();
    let result = evaluate(
        &evaluator,
        vec![SkyKey::new(key_f.clone())],
        true,
        Arc::new(StoredEventHandler::new()),
    );
    assert!(!result.has_error(), "{:?}", result);
    let e = result.get(&key_f).unwrap();
    assert_eq!(
        e.path_to_unbounded_ancestor_symlink_expansion_chain()
            .unwrap(),
        &[f.rooted_path("f"), f.rooted_path("e")]
    );
    assert_eq!(
        e.unbounded_ancestor_symlink_expansion_chain().unwrap(),
        &[
            f.rooted_path("a/b/c"),
            f.rooted_path("d/b"),
            f.rooted_path("a/b")
        ]
    );
}

#[test]
fn test_infinite_symlink_expansion_symlink_to_referrer_to_ancestor_levels_of_directory_symlinks() {
    let f = Fixture::new();
    f.symlink("dir1/a", "../dir2");
    f.symlink("dir2/b", "../dir1");

    let rooted_path_dir1_ab = RootedPath::to_rooted_path(
        f.pkg_root.clone(),
        &f.pkg_root.relativize(&f.path("dir1/a/b")),
    );
    let key_dir1_ab = FileKey::key(rooted_path_dir1_ab);

    let evaluator = f.setup.evaluator();
    let result = evaluate(
        &evaluator,
        vec![SkyKey::new(key_dir1_ab)],
        true,
        Arc::new(StoredEventHandler::new()),
    );
    assert!(!result.has_error(), "{:?}", result);
}

#[test]
fn test_multiple_levels_of_directory_symlinks_clean() {
    let f = Fixture::new();
    f.symlink("a/b/c", "../c");
    let abcd = f.path("a/b/c/d");
    f.symlink("a/c/d", "../d");
    assert!(f.value_for_path(&abcd).is_symlink());
}

#[test]
fn test_child_of_nonexistent_parent() {
    let f = Fixture::new();
    let ancestor = f.directory("this/is/an/ancestor");
    let parent = ancestor.get_child("parent");
    let child = parent.get_child("child");
    assert!(!f.value_for_path(&parent).exists());
    assert!(!f.value_for_path(&child).exists());
}

/// Java `isInstanceOf(DifferentRealPathFileValueWithStoredChain.class)`: the class and its
/// subclasses (the symlink and unbounded-expansion variants).
fn is_instance_of_different_real_path_with_stored_chain(v: &FileValue) -> bool {
    matches!(
        v,
        FileValue::DifferentRealPathWithStoredChain { .. }
            | FileValue::DifferentRealPathWithUnboundedAncestorExpansion { .. }
            | FileValue::SymlinkWithStoredChain { .. }
            | FileValue::SymlinkWithUnboundedAncestorExpansion { .. }
    )
}

/// Java `isInstanceOf(DifferentRealPathFileValueWithoutStoredChain.class)`.
fn is_instance_of_different_real_path_without_stored_chain(v: &FileValue) -> bool {
    matches!(
        v,
        FileValue::DifferentRealPathWithoutStoredChain { .. }
            | FileValue::SymlinkWithoutStoredChain { .. }
    )
}

#[test]
fn test_logical_chain_during_resolution_directory_simple_symlink() {
    let f = Fixture::new();
    f.symlink("a", "b");
    f.symlink("b", "c");
    f.directory("c");
    let rooted_path = f.rooted_path("a");
    let file_value = f.value_for_rooted_path(rooted_path.clone());
    assert!(matches!(
        file_value,
        FileValue::SymlinkWithStoredChain { .. }
    ));
    assert_eq!(
        *file_value.get_unresolved_link_target(),
        PathFragment::create("b")
    );
    assert_eq!(
        file_value.logical_chain_during_resolution(&rooted_path),
        vec![f.rooted_path("a"), f.rooted_path("b"), f.rooted_path("c")]
    );
}

#[test]
fn test_logical_chain_during_resolution_directory_simple_ancestor_symlink() {
    let f = Fixture::new();
    f.symlink("a", "b");
    f.symlink("b", "c");
    f.directory("c/d");
    let rooted_path = f.rooted_path("a/d");
    let file_value = f.value_for_rooted_path(rooted_path.clone());
    assert!(is_instance_of_different_real_path_with_stored_chain(
        &file_value
    ));
    assert_eq!(
        file_value.real_rooted_path(&rooted_path),
        f.rooted_path("c/d")
    );
    assert_eq!(
        file_value.logical_chain_during_resolution(&rooted_path),
        vec![
            f.rooted_path("a/d"),
            f.rooted_path("b/d"),
            f.rooted_path("c/d")
        ]
    );
}

#[test]
fn test_logical_chain_during_resolution_file_simple_symlink() {
    let f = Fixture::new();
    f.symlink("a", "b");
    f.symlink("b", "c");
    f.file("c");
    let rooted_path = f.rooted_path("a");
    let file_value = f.value_for_rooted_path(rooted_path.clone());
    assert!(matches!(
        file_value,
        FileValue::SymlinkWithoutStoredChain { .. }
    ));
    assert_eq!(
        *file_value.get_unresolved_link_target(),
        PathFragment::create("b")
    );
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        file_value.logical_chain_during_resolution(&rooted_path)
    }));
    assert!(r.is_err(), "expected IllegalStateException");
}

#[test]
fn test_logical_chain_during_resolution_file_simple_ancestor_symlink() {
    let f = Fixture::new();
    f.symlink("a", "b");
    f.symlink("b", "c");
    f.file("c/d");
    let rooted_path = f.rooted_path("a/d");
    let file_value = f.value_for_rooted_path(rooted_path.clone());
    assert!(is_instance_of_different_real_path_without_stored_chain(
        &file_value
    ));
    assert_eq!(
        file_value.real_rooted_path(&rooted_path),
        f.rooted_path("c/d")
    );
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        file_value.logical_chain_during_resolution(&rooted_path)
    }));
    assert!(r.is_err(), "expected IllegalStateException");
}

#[test]
fn test_logical_chain_during_resolution_complicated() {
    let f = Fixture::new();
    f.symlink("a", "b");
    f.symlink("b", "c");
    f.directory("c");
    f.symlink("c/d", "../e/f");
    f.symlink("e", "g");
    f.directory("g");
    f.symlink("g/f", "../h");
    f.directory("h");
    let rooted_path = f.rooted_path("a/d");
    let file_value = f.value_for_rooted_path(rooted_path.clone());
    assert!(is_instance_of_different_real_path_with_stored_chain(
        &file_value
    ));
    assert_eq!(
        file_value.real_rooted_path(&rooted_path),
        f.rooted_path("h")
    );
    assert_eq!(
        file_value.logical_chain_during_resolution(&rooted_path),
        vec![
            f.rooted_path("a/d"),
            f.rooted_path("b/d"),
            f.rooted_path("c/d"),
            f.rooted_path("e/f"),
            f.rooted_path("g/f"),
            f.rooted_path("h"),
        ]
    );
}

#[test]
fn file_key_debug_matches_java_to_string() {
    let f = Fixture::new();
    let key = f.sky_key("a/b");
    assert_eq!(format!("{:?}", key), format!("FILE:[{}]/[a/b]", f.pkg_root));
    assert_eq!(FileKey::FUNCTION.get_name(), "FILE");
}

macro_rules! ignored {
    ($($name:ident: $reason:literal,)*) => {
        $(
            #[test]
            #[ignore = $reason]
            fn $name() {}
        )*
    };
}

ignored! {
    test_simple_independent_files: "needs incremental re-evaluation (differencer)",
    test_simple_symlink: "needs incremental re-evaluation (differencer)",
    test_transitive_symlink: "needs incremental re-evaluation (differencer)",
    test_file_under_broken_directory_symlink: "needs incremental re-evaluation (differencer)",
    test_file_under_directory_symlink: "needs incremental re-evaluation (differencer)",
    test_symlink_in_directory: "needs incremental re-evaluation (differencer)",
    test_relative_symlink: "needs incremental re-evaluation (differencer)",
    test_double_relative_symlink: "needs incremental re-evaluation (differencer)",
    test_external_relative_symlink: "needs incremental re-evaluation (differencer)",
    test_absolute_symlink: "needs incremental re-evaluation (differencer)",
    test_absolute_symlink_to_external: "needs incremental re-evaluation (differencer)",
    test_symlink_as_ancestor: "needs incremental re-evaluation (differencer)",
    test_symlink_as_ancestor_nested: "needs incremental re-evaluation (differencer)",
    test_two_symlinks_in_ancestors: "needs incremental re-evaluation (differencer)",
    test_broken_symlink: "needs incremental re-evaluation (differencer)",
    test_broken_directory_symlink: "needs incremental re-evaluation (differencer)",
    test_empty_file: "needs Java's in-memory file system (fast digests, manual clock)",
    test_unreadable_file_with_no_fast_digest: "needs Java's in-memory file system (injected failures)",
    test_unreadable_file_with_fast_digest: "needs Java's in-memory file system (fast digests)",
    test_file_modification_digest: "needs incremental re-evaluation (differencer)",
    test_mod_time_vs_digest: "needs incremental re-evaluation (differencer)",
    test_file_deletion: "needs incremental re-evaluation (differencer)",
    test_file_type_change: "needs incremental re-evaluation (differencer)",
    test_symlink_target_change: "needs incremental re-evaluation (differencer)",
    test_symlink_target_contents_change_c_time: "needs incremental re-evaluation (differencer)",
    test_symlink_target_contents_change_digest: "needs incremental re-evaluation (differencer)",
    test_symlink_across_package_roots: "needs incremental re-evaluation (differencer)",
    test_files_outside_root_is_re_evaluated: "needs incremental re-evaluation (differencer)",
    test_files_outside_root_when_external_assumed_non_existent_and_immutable: "needs incremental re-evaluation (differencer)",
    test_absolute_symlinks_to_files_outside_root_when_external_assumed_non_existent_and_immutable: "needs incremental re-evaluation (differencer)",
    test_relative_symlinks_to_files_outside_root_when_external_assumed_non_existent_and_immutable: "needs incremental re-evaluation (differencer)",
    test_absolute_symlinks_back_into_sources_ok_when_external_disallowed: "needs incremental re-evaluation (differencer)",
    test_size: "needs Java's in-memory file system (manual clock)",
    test_digest: "needs Java's in-memory file system (fast digests)",
    test_doesnt_stat_child_if_parent_doesnt_exist: "needs Java's in-memory file system (stat recording)",
    test_filesystem_inconsistencies_get_fast_digest: "needs Java's in-memory file system (injected failures)",
    test_filesystem_inconsistencies_get_fast_digest_and_is_readable_failure: "needs Java's in-memory file system (injected failures)",
    test_serialization: "serialization is not ported",
    test_injection_over_io_exception: "needs incremental re-evaluation (injection over an error)",
    test_multiple_levels_of_directory_symlinks_incremental: "needs incremental re-evaluation (differencer)",
    test_file_access_exception: "needs Java's in-memory file system (injected failures)",
}
