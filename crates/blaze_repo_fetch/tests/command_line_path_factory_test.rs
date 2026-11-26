//! Port of `CommandLinePathFactoryTest`.
//!
//! Java runs the tests on an `InMemoryFileSystem`; here the path lookups run on the real file
//! system under a temporary directory, so Java's absolute paths of executables (`/bin/true`) are
//! prefixed with that directory. Paths that are only computed, never touched, keep Java's form.
// Java: src/test/java/com/google/devtools/build/lib/runtime/CommandLinePathFactoryTest.java

use blaze_repo_fetch::authandtls::credentialhelper::CommandLinePathFactory;
use blaze_repo_fetch::errors::IoExceptionKind;
use blaze_vfs::unix_file_system::UnixFileSystem;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path};
use indexmap::IndexMap;

struct Fixture {
    _dir: tempfile::TempDir,
    filesystem: FileSystemRef,
    root: String,
}

fn fixture() -> Fixture {
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

impl Fixture {
    fn get_path(&self, path: &str) -> Path {
        Path::create_from_str(path, &self.filesystem)
    }

    /// A path of the in-memory file system of the Java test, under the temporary directory.
    fn local(&self, path: &str) -> String {
        format!("{}{}", self.root, path)
    }

    fn create_executable(&self, path: &str) {
        let path = self.get_path(&self.local(path));
        path.get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        blaze_repo_fetch::fs_utils::write_content(&path, b"").unwrap();
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
}

fn env(entries: &[(&str, &str)]) -> IndexMap<String, String> {
    entries
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn assert_kind(result: Result<Path, blaze_repo_fetch::errors::IoException>, kind: IoExceptionKind) {
    match result {
        Ok(path) => panic!("expected {kind:?}, got {path}"),
        Err(e) => assert_eq!(e.kind, kind, "{e:?}"),
    }
}

#[test]
#[should_panic(expected = "IllegalArgumentException")]
fn empty_path_is_rejected() {
    let f = fixture();
    let factory = f.factory(&[]);

    let _ = factory.create(&env(&[]), "");
}

#[test]
fn create_from_absolute_path() {
    let f = fixture();
    let factory = f.factory(&[]);

    assert_eq!(
        factory.create(&env(&[]), "/absolute/path/1").unwrap(),
        f.get_path("/absolute/path/1")
    );
    assert_eq!(
        factory.create(&env(&[]), "/absolute/path/2").unwrap(),
        f.get_path("/absolute/path/2")
    );
}

#[test]
fn create_with_named_root() {
    let f = fixture();
    let factory = f.factory(&[
        ("workspace", "/path/to/workspace"),
        ("output_base", "/path/to/output/base"),
    ]);

    assert_eq!(
        factory.create(&env(&[]), "/absolute/path/1").unwrap(),
        f.get_path("/absolute/path/1")
    );
    assert_eq!(
        factory.create(&env(&[]), "/absolute/path/2").unwrap(),
        f.get_path("/absolute/path/2")
    );

    assert_eq!(
        factory.create(&env(&[]), "%workspace%/foo").unwrap(),
        f.get_path("/path/to/workspace/foo")
    );
    assert_eq!(
        factory.create(&env(&[]), "%workspace%/foo/bar").unwrap(),
        f.get_path("/path/to/workspace/foo/bar")
    );

    assert_eq!(
        factory.create(&env(&[]), "%output_base%/foo").unwrap(),
        f.get_path("/path/to/output/base/foo")
    );
    assert_eq!(
        factory.create(&env(&[]), "%output_base%/foo/bar").unwrap(),
        f.get_path("/path/to/output/base/foo/bar")
    );

    assert_eq!(
        factory.create(&env(&[]), "%workspace%//foo//bar").unwrap(),
        f.get_path("/path/to/workspace/foo/bar")
    );
}

#[test]
fn path_leaking_outside_of_root() {
    let f = fixture();
    let factory = f.factory(&[("a", "/path/to/a")]);

    assert_kind(
        factory.create(&env(&[]), "%a%/../foo"),
        IoExceptionKind::CommandLinePathFactoryException,
    );
    assert_kind(
        factory.create(&env(&[]), "%a%/b/../.."),
        IoExceptionKind::CommandLinePathFactoryException,
    );
}

#[test]
fn unknown_root() {
    let f = fixture();
    let factory = f.factory(&[("a", "/path/to/a")]);

    assert_kind(
        factory.create(&env(&[]), "%workspace%/foo"),
        IoExceptionKind::CommandLinePathFactoryException,
    );
    assert_kind(
        factory.create(&env(&[]), "%output_base%/foo"),
        IoExceptionKind::CommandLinePathFactoryException,
    );
}

#[test]
fn relative_path_with_multiple_segments() {
    let f = fixture();
    let factory = f.factory(&[]);

    assert_kind(
        factory.create(&env(&[]), "a/b"),
        IoExceptionKind::CommandLinePathFactoryException,
    );
    assert_kind(
        factory.create(&env(&[]), "a/b/c/d"),
        IoExceptionKind::CommandLinePathFactoryException,
    );
}

#[test]
fn path_lookup() {
    let f = fixture();
    let factory = f.factory(&[]);

    f.create_executable("/bin/true");
    f.create_executable("/bin/false");
    f.create_executable("/usr/bin/foo-bar.exe");
    f.create_executable("/usr/local/bin/baz");
    f.create_executable("/home/yannic/bin/abc");
    f.create_executable("/home/yannic/bin/true");

    let path_value = [
        f.local("/bin"),
        f.local("/usr/bin"),
        f.local("/usr/local/bin"),
        f.local("/home/yannic/bin"),
    ]
    .join(":");
    let path = env(&[("PATH", &path_value)]);
    assert_eq!(
        factory.create(&path, "true").unwrap(),
        f.get_path(&f.local("/bin/true"))
    );
    assert_eq!(
        factory.create(&path, "false").unwrap(),
        f.get_path(&f.local("/bin/false"))
    );
    assert_eq!(
        factory.create(&path, "foo-bar.exe").unwrap(),
        f.get_path(&f.local("/usr/bin/foo-bar.exe"))
    );
    assert_eq!(
        factory.create(&path, "baz").unwrap(),
        f.get_path(&f.local("/usr/local/bin/baz"))
    );
    assert_eq!(
        factory.create(&path, "abc").unwrap(),
        f.get_path(&f.local("/home/yannic/bin/abc"))
    );

    // `.exe` is required.
    assert_kind(
        factory.create(&path, "foo-bar"),
        IoExceptionKind::FileNotFoundException,
    );
}

#[test]
fn path_lookup_with_undefined_path() {
    let f = fixture();
    let factory = f.factory(&[]);

    assert_kind(
        factory.create(&env(&[]), "a"),
        IoExceptionKind::FileNotFoundException,
    );
    assert_kind(
        factory.create(&env(&[]), "foo"),
        IoExceptionKind::FileNotFoundException,
    );
}

#[test]
fn path_lookup_with_non_existing_directory_on_path() {
    let f = fixture();
    let factory = f.factory(&[]);

    assert_kind(
        factory.create(&env(&[("PATH", &f.local("/does/not/exist"))]), "a"),
        IoExceptionKind::FileNotFoundException,
    );
}

#[test]
fn path_lookup_with_existing_and_non_existing_directory_on_path() {
    let f = fixture();
    let factory = f.factory(&[]);

    f.create_executable("/bin/foo");
    f.create_executable("/usr/bin/bar");
    let path_value = [
        f.local("/bin"),
        f.local("/does/not/exist"),
        f.local("/usr/bin"),
    ]
    .join(":");
    assert_kind(
        factory.create(&env(&[("PATH", &path_value)]), "a"),
        IoExceptionKind::FileNotFoundException,
    );
}

#[test]
fn path_lookup_with_invalid_path() {
    let f = fixture();
    let factory = f.factory(&[]);

    f.create_executable("/bin/true");
    let path_value = ["".to_string(), ".".to_string(), f.local("/bin")].join(":");
    assert_eq!(
        factory
            .create(&env(&[("PATH", &path_value)]), "true")
            .unwrap(),
        f.get_path(&f.local("/bin/true"))
    );
}
