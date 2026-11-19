//! Port of `src/test/java/com/google/devtools/build/lib/runtime/CommandLinePathFactoryTest.java`.
//!
//! Java runs the tests on an `InMemoryFileSystem` (not ported); here they run on the real file
//! system under a temporary directory, which prefixes Java's absolute paths (including the
//! entries of `PATH`).

use blaze_repo_fetch::runtime::command_line_path_factory::{
    CommandLinePathFactory, CommandLinePathFactoryError,
};
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};
use indexmap::IndexMap;

struct Fixture {
    _dir: tempfile::TempDir,
    filesystem: FileSystemRef,
    root: String,
}

impl Fixture {
    // Java: CommandLinePathFactoryTest.prepareFilesystem
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        Fixture {
            _dir: dir,
            filesystem: UnixFileSystem::create(DigestHashFunction::Sha256),
            root,
        }
    }

    /// `filesystem.getPath(path)` for one of Java's absolute paths.
    fn get_path(&self, path: &str) -> Path {
        Path::create_from_str(&format!("{}{}", self.root, path), &self.filesystem)
    }

    fn create_executable(&self, path: &str) {
        let path = self.get_path(path);
        path.get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        // Just create an empty file, nothing to do.
        drop(path.get_output_stream().unwrap());
        path.set_executable(true).unwrap();
    }

    fn factory(&self, roots: &[(&str, &str)]) -> CommandLinePathFactory {
        CommandLinePathFactory::new(
            self.filesystem.clone(),
            roots
                .iter()
                .map(|(name, path)| (name.to_string(), self.get_path(path)))
                .collect(),
        )
    }

    /// `PATH_JOINER.join(...)` of Java's absolute directories (empty and relative entries
    /// unchanged).
    fn path_env(&self, entries: &[&str]) -> IndexMap<String, String> {
        let joined = entries
            .iter()
            .map(|e| {
                if e.starts_with('/') {
                    format!("{}{}", self.root, e)
                } else {
                    e.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(":");
        env(&[("PATH", &joined)])
    }
}

fn env(entries: &[(&str, &str)]) -> IndexMap<String, String> {
    entries
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn is_command_line_path_factory_exception<T>(r: Result<T, CommandLinePathFactoryError>) -> bool {
    matches!(
        r,
        Err(CommandLinePathFactoryError::CommandLinePathFactoryException(_))
    )
}

fn is_file_not_found<T>(r: Result<T, CommandLinePathFactoryError>) -> bool {
    matches!(
        r,
        Err(CommandLinePathFactoryError::FileNotFoundException(_))
    )
}

#[test]
#[should_panic(expected = "IllegalArgumentException")]
fn empty_path_is_rejected() {
    let f = Fixture::new();
    let factory = f.factory(&[]);

    let _ = factory.create(&env(&[]), "");
}

#[test]
fn create_from_absolute_path() {
    let f = Fixture::new();
    let factory = f.factory(&[]);
    let p1 = f.get_path("/absolute/path/1");
    let p2 = f.get_path("/absolute/path/2");

    assert_eq!(
        factory.create(&env(&[]), p1.get_path_string()),
        Ok(p1.clone())
    );
    assert_eq!(
        factory.create(&env(&[]), p2.get_path_string()),
        Ok(p2.clone())
    );
}

#[test]
fn create_with_named_root() {
    let f = Fixture::new();
    let factory = f.factory(&[
        ("workspace", "/path/to/workspace"),
        ("output_base", "/path/to/output/base"),
    ]);
    let p1 = f.get_path("/absolute/path/1");
    let p2 = f.get_path("/absolute/path/2");

    assert_eq!(
        factory.create(&env(&[]), p1.get_path_string()),
        Ok(p1.clone())
    );
    assert_eq!(
        factory.create(&env(&[]), p2.get_path_string()),
        Ok(p2.clone())
    );

    assert_eq!(
        factory.create(&env(&[]), "%workspace%/foo"),
        Ok(f.get_path("/path/to/workspace/foo"))
    );
    assert_eq!(
        factory.create(&env(&[]), "%workspace%/foo/bar"),
        Ok(f.get_path("/path/to/workspace/foo/bar"))
    );

    assert_eq!(
        factory.create(&env(&[]), "%output_base%/foo"),
        Ok(f.get_path("/path/to/output/base/foo"))
    );
    assert_eq!(
        factory.create(&env(&[]), "%output_base%/foo/bar"),
        Ok(f.get_path("/path/to/output/base/foo/bar"))
    );

    assert_eq!(
        factory.create(&env(&[]), "%workspace%//foo//bar"),
        Ok(f.get_path("/path/to/workspace/foo/bar"))
    );
}

#[test]
fn path_leaking_outside_of_root() {
    let f = Fixture::new();
    let factory = f.factory(&[("a", "/path/to/a")]);

    assert!(is_command_line_path_factory_exception(
        factory.create(&env(&[]), "%a%/../foo")
    ));
    assert!(is_command_line_path_factory_exception(
        factory.create(&env(&[]), "%a%/b/../..")
    ));
}

#[test]
fn unknown_root() {
    let f = Fixture::new();
    let factory = f.factory(&[("a", "/path/to/a")]);

    assert!(is_command_line_path_factory_exception(
        factory.create(&env(&[]), "%workspace%/foo")
    ));
    assert!(is_command_line_path_factory_exception(
        factory.create(&env(&[]), "%output_base%/foo")
    ));
}

#[test]
fn relative_path_with_multiple_segments() {
    let f = Fixture::new();
    let factory = f.factory(&[]);

    assert!(is_command_line_path_factory_exception(
        factory.create(&env(&[]), "a/b")
    ));
    assert!(is_command_line_path_factory_exception(
        factory.create(&env(&[]), "a/b/c/d")
    ));
}

#[test]
fn path_lookup() {
    let f = Fixture::new();
    let factory = f.factory(&[]);

    f.create_executable("/bin/true");
    f.create_executable("/bin/false");
    f.create_executable("/usr/bin/foo-bar.exe");
    f.create_executable("/usr/local/bin/baz");
    f.create_executable("/home/yannic/bin/abc");
    f.create_executable("/home/yannic/bin/true");

    let path = f.path_env(&["/bin", "/usr/bin", "/usr/local/bin", "/home/yannic/bin"]);
    assert_eq!(factory.create(&path, "true"), Ok(f.get_path("/bin/true")));
    assert_eq!(factory.create(&path, "false"), Ok(f.get_path("/bin/false")));
    assert_eq!(
        factory.create(&path, "foo-bar.exe"),
        Ok(f.get_path("/usr/bin/foo-bar.exe"))
    );
    assert_eq!(
        factory.create(&path, "baz"),
        Ok(f.get_path("/usr/local/bin/baz"))
    );
    assert_eq!(
        factory.create(&path, "abc"),
        Ok(f.get_path("/home/yannic/bin/abc"))
    );

    // `.exe` is required.
    assert!(is_file_not_found(factory.create(&path, "foo-bar")));
}

#[test]
fn path_lookup_with_undefined_path() {
    let f = Fixture::new();
    let factory = f.factory(&[]);

    assert!(is_file_not_found(factory.create(&env(&[]), "a")));
    assert!(is_file_not_found(factory.create(&env(&[]), "foo")));
}

#[test]
fn path_lookup_with_non_existing_directory_on_path() {
    let f = Fixture::new();
    let factory = f.factory(&[]);

    assert!(is_file_not_found(
        factory.create(&f.path_env(&["/does/not/exist"]), "a")
    ));
}

#[test]
fn path_lookup_with_existing_and_non_existing_directory_on_path() {
    let f = Fixture::new();
    let factory = f.factory(&[]);

    f.create_executable("/bin/foo");
    f.create_executable("/usr/bin/bar");
    assert!(is_file_not_found(factory.create(
        &f.path_env(&["/bin", "/does/not/exist", "/usr/bin"]),
        "a"
    )));
}

#[test]
fn path_lookup_with_invalid_path() {
    let f = Fixture::new();
    let factory = f.factory(&[]);

    f.create_executable("/bin/true");
    let path = f.path_env(&["", ".", "/bin"]);
    assert_eq!(factory.create(&path, "true"), Ok(f.get_path("/bin/true")));
}

// The tests below have no Java counterpart: they pin the error texts and the other checks of
// `create`.

#[test]
fn error_messages() {
    let f = Fixture::new();
    let factory = f.factory(&[("a", "/path/to/a")]);
    let message = |env: &IndexMap<String, String>, value: &str| {
        factory.create(env, value).unwrap_err().get_message()
    };

    assert_eq!(
        message(&env(&[]), "%a%/../foo"),
        "Path '%a%/../foo' must not contain any uplevel references ('..')"
    );
    assert_eq!(message(&env(&[]), "%b%/foo"), "Unknown root b");
    assert_eq!(
        message(&env(&[]), "a/b"),
        "Path 'a/b' must either be absolute or not contain any path separators"
    );
    assert_eq!(
        message(&env(&[("PATH", "/does/not/exist:")]), "helper"),
        "Could not find file with name 'helper' on PATH '/does/not/exist:'"
    );
}

#[test]
fn path_lookup_skips_non_executable_files_and_directories() {
    let f = Fixture::new();
    let factory = f.factory(&[]);

    let plain = f.get_path("/bin/plain");
    plain
        .get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    drop(plain.get_output_stream().unwrap());
    plain.set_executable(false).unwrap();
    f.get_path("/bin/dir")
        .create_directory_and_parents()
        .unwrap();
    f.create_executable("/usr/bin/plain");

    let path = f.path_env(&["/bin", "/usr/bin"]);
    assert_eq!(
        factory.create(&path, "plain"),
        Ok(f.get_path("/usr/bin/plain"))
    );
    assert!(is_file_not_found(factory.create(&path, "dir")));
}

#[test]
fn create_for_directories_has_workspace_and_install_base_roots() {
    let f = Fixture::new();
    let factory = CommandLinePathFactory::create_for_directories(
        f.filesystem.clone(),
        Some(f.get_path("/workspace")),
        Some(f.get_path("/install")),
    );
    assert_eq!(
        factory.create(&env(&[]), "%workspace%/tools/helper"),
        Ok(f.get_path("/workspace/tools/helper"))
    );
    assert_eq!(
        factory.create(&env(&[]), "%install_base%/helper"),
        Ok(f.get_path("/install/helper"))
    );
}
