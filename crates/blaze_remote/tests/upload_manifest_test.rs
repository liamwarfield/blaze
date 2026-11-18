#![cfg(feature = "wip")]
// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
// The deprecated ActionResult symlink fields are still written by Java and asserted by the tests.
#![allow(deprecated)]
//! Port of `src/test/java/com/google/devtools/build/lib/remote/UploadManifestTest.java`.
//!
//! Java runs the tests on an `InMemoryFileSystem` rooted at `/execroot`; here they run on the
//! real file system under a temporary directory, so Java's absolute paths (`/execroot/target`)
//! are prefixed with that directory. Java mocks `Path` for the special files because Bazel's file
//! systems cannot create them; the real file system can, so the port makes FIFOs.
//!
//! Java passes an `ActionResult.Builder` to the manifest and reads it back with
//! `result.build()`; the port's manifest owns the builder, so the tests read
//! `UploadManifest::get_action_result()`.

use std::sync::Arc;

use blaze_exec::actions::ExecException;
use blaze_exec::exec::action_execution_context::SpawnCacheError;
use blaze_remote::common::{DefaultRemotePathResolver, RemotePathResolver};
use blaze_remote::reapi::{
    ActionResult, Digest, Directory, DirectoryNode, FileNode, OutputDirectory, OutputFile,
    OutputSymlink, SymlinkNode, Tree,
};
use blaze_remote::upload_manifest::UploadManifest;
use blaze_remote::util::DigestUtil;
use blaze_repo_fetch::fs_utils;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, PathFragment, UnixFileSystem};
use indexmap::IndexMap;

/// The fields of the Java test class, as `setUp` leaves them.
struct Fixture {
    _dir: tempfile::TempDir,
    _fs: FileSystemRef,
    digest_util: DigestUtil,
    exec_root: Path,
    remote_path_resolver: Arc<dyn RemotePathResolver>,
}

impl Fixture {
    // Java: com/google/devtools/build/lib/remote/UploadManifestTest.java:193 UploadManifestTest.setUp
    fn set_up() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let exec_root = Path::create_from_str(&format!("{root}/execroot"), &fs);
        exec_root.create_directory_and_parents().unwrap();

        let remote_path_resolver: Arc<dyn RemotePathResolver> =
            Arc::new(DefaultRemotePathResolver::new(exec_root.clone()));
        Fixture {
            _dir: dir,
            _fs: fs,
            digest_util: DigestUtil::new(DigestHashFunction::Sha256),
            exec_root,
            remote_path_resolver,
        }
    }

    /// `new UploadManifest(digestUtil, remotePathResolver, result, allowAbsoluteSymlinks)`.
    fn new_upload_manifest(&self, allow_absolute_symlinks: bool) -> UploadManifest {
        UploadManifest::new(
            self.digest_util.clone(),
            self.remote_path_resolver.clone(),
            ActionResult::default(),
            allow_absolute_symlinks,
        )
    }

    /// `new UploadManifest(digestUtil, remotePathResolver, result, allowAbsoluteSymlinks,
    /// preserveExecutableBit)`.
    fn new_upload_manifest_with_preserve_executable_bit(
        &self,
        allow_absolute_symlinks: bool,
        preserve_executable_bit: bool,
    ) -> UploadManifest {
        UploadManifest::new_with_preserve_executable_bit(
            self.digest_util.clone(),
            self.remote_path_resolver.clone(),
            ActionResult::default(),
            allow_absolute_symlinks,
            preserve_executable_bit,
        )
    }

    /// The path string Java has for `/execroot/<rel>`.
    fn execroot_path_string(&self, rel: &str) -> String {
        self.exec_root
            .get_relative_str(rel)
            .get_path_string()
            .to_string()
    }

    // Tests to verify that files with an unsupported type (collectively, "special files") are
    // rejected.

    // Java: com/google/devtools/build/lib/remote/UploadManifestTest.java:737 UploadManifestTest.createSpecialFile
    fn create_special_file(&self, exec_path: &str) -> Path {
        let special = self.exec_root.get_relative_str(exec_path);
        let c_path = std::ffi::CString::new(special.get_path_string()).unwrap();
        // SAFETY: `c_path` is a valid NUL-terminated string that outlives the call.
        let rc = unsafe { libc::mkfifo(c_path.as_ptr(), 0o644) };
        assert_eq!(
            rc,
            0,
            "mkfifo({special}): {}",
            std::io::Error::last_os_error()
        );
        special
    }

    // Java: com/google/devtools/build/lib/remote/UploadManifestTest.java:747 UploadManifestTest.createSymlinkToSpecialFile
    fn create_symlink_to_special_file(&self, exec_path: &str, target: &str) -> Path {
        let link = self.exec_root.get_relative_str(exec_path);
        link.create_symbolic_link_to_fragment(&PathFragment::create(target))
            .unwrap();
        link
    }

    // Java: com/google/devtools/build/lib/remote/UploadManifestTest.java:757 UploadManifestTest.createDirectoryWithSpecialFile
    fn create_directory_with_special_file(&self, dir_exec_path: &str, special_name: &str) -> Path {
        let dir = self.exec_root.get_relative_str(dir_exec_path);
        dir.create_directory().unwrap();
        let _special = self.create_special_file(&format!("{dir_exec_path}/{special_name}"));
        dir
    }

    // Java: com/google/devtools/build/lib/remote/UploadManifestTest.java:771 UploadManifestTest.createDirectoryWithSymlinkToSpecialFile
    fn create_directory_with_symlink_to_special_file(
        &self,
        dir_exec_path: &str,
        link_name: &str,
        special_name: &str,
    ) -> Path {
        // PORT: Java's mocked directory lists only the link; the special file the link points to
        // has to exist on a real file system, so it lives next to the directory.
        let special_exec_path = format!("{dir_exec_path}-{special_name}");
        let _unused_special = self.create_special_file(&special_exec_path);
        let dir = self.exec_root.get_relative_str(dir_exec_path);
        dir.create_directory().unwrap();
        let _link = self.create_symlink_to_special_file(
            &format!("{dir_exec_path}/{link_name}"),
            self.exec_root
                .get_relative_str(&special_exec_path)
                .get_path_string(),
        );
        dir
    }
}

/// `FileSystemUtils.writeContent(path, bytes)`.
fn write_content(path: &Path, content: &[u8]) {
    fs_utils::write_content(path, content).unwrap();
}

/// `StringEncoding.unicodeToInternal(s)`, as the string Blaze holds for that internal string.
fn unicode_to_internal(s: &str) -> String {
    if blaze_util::string_encoding::strings_are_internal() {
        blaze_util::string_encoding::unicode_to_internal(s).into_owned()
    } else {
        s.to_string()
    }
}

/// `assertThat(um.getDigestToFile()).containsExactly(...)`.
fn assert_digest_to_file(um: &UploadManifest, expected: &[(&Digest, &Path)]) {
    let expected: IndexMap<Digest, Path> = expected
        .iter()
        .map(|(digest, path)| ((*digest).clone(), (*path).clone()))
        .collect();
    assert_eq!(um.get_digest_to_file(), &expected);
}

fn output_file(path: &str, digest: &Digest, is_executable: bool) -> OutputFile {
    OutputFile {
        path: path.to_string(),
        digest: Some(digest.clone()),
        is_executable,
        ..Default::default()
    }
}

fn output_symlink(path: &str, target: &str) -> OutputSymlink {
    OutputSymlink {
        path: path.to_string(),
        target: target.to_string(),
        ..Default::default()
    }
}

fn output_directory(path: &str, tree_digest: &Digest) -> OutputDirectory {
    OutputDirectory {
        path: path.to_string(),
        tree_digest: Some(tree_digest.clone()),
        is_topologically_sorted: true,
        ..Default::default()
    }
}

fn file_node(name: &str, digest: &Digest, is_executable: bool) -> FileNode {
    FileNode {
        name: name.to_string(),
        digest: Some(digest.clone()),
        is_executable,
        ..Default::default()
    }
}

fn symlink_node(name: &str, target: &str) -> SymlinkNode {
    SymlinkNode {
        name: name.to_string(),
        target: target.to_string(),
        ..Default::default()
    }
}

fn directory_node(name: &str, digest: &Digest) -> DirectoryNode {
    DirectoryNode {
        name: name.to_string(),
        digest: Some(digest.clone()),
    }
}

/// `assertThrows(IOException.class, ...)`.
fn expect_io_exception(result: Result<(), SpawnCacheError>) -> std::io::Error {
    match result {
        Err(SpawnCacheError::Io(e)) => e,
        other => panic!("expected an IOException, got {other:?}"),
    }
}

/// `assertThrows(UserExecException.class, ...)`: the message of the exception.
fn expect_user_exec_exception(result: Result<(), SpawnCacheError>) -> String {
    match result {
        Err(SpawnCacheError::Exec(ExecException::User(e))) => e.message,
        other => panic!("expected a UserExecException, got {other:?}"),
    }
}

#[test]
fn action_result_absolute_file_symlink_as_file() {
    let f = Fixture::set_up();
    let link = f.exec_root.get_relative_str("link");
    let target = f.exec_root.get_relative_str("target");
    write_content(&target, &[1, 2, 3, 4, 5]);
    link.create_symbolic_link(&target).unwrap();

    let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ false);
    um.add_files(std::slice::from_ref(&link)).unwrap();
    let digest = f.digest_util.compute_path(&target).unwrap();
    assert_digest_to_file(&um, &[(&digest, &link)]);

    let expected_result = ActionResult {
        output_files: vec![output_file("link", &digest, true)],
        ..Default::default()
    };
    assert_eq!(um.get_action_result(), expected_result);
}

#[test]
fn action_result_absolute_directory_symlink_as_directory() {
    let f = Fixture::set_up();
    let dir = f.exec_root.get_relative_str("dir");
    dir.create_directory().unwrap();
    let foo = f.exec_root.get_relative_str("dir/foo");
    write_content(&foo, &[1, 2, 3, 4, 5]);
    let link = f.exec_root.get_relative_str("link");
    link.create_symbolic_link(&dir).unwrap();

    let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ false);
    um.add_files(std::slice::from_ref(&link)).unwrap();
    let digest = f.digest_util.compute_path(&foo).unwrap();
    assert_digest_to_file(&um, &[(&digest, &f.exec_root.get_relative_str("link/foo"))]);

    let tree = Tree {
        root: Some(Directory {
            files: vec![file_node("foo", &digest, true)],
            ..Default::default()
        }),
        ..Default::default()
    };
    let tree_digest = f.digest_util.compute_message(&tree);

    let expected_result = ActionResult {
        output_directories: vec![output_directory("link", &tree_digest)],
        ..Default::default()
    };
    assert_eq!(um.get_action_result(), expected_result);
}

#[test]
fn action_result_relative_file_symlink_as_symlink() {
    let f = Fixture::set_up();
    let link = f.exec_root.get_relative_str("link");
    let target = f.exec_root.get_relative_str("target");
    write_content(&target, &[1, 2, 3, 4, 5]);
    link.create_symbolic_link_to_fragment(&target.relative_to(&f.exec_root))
        .unwrap();

    let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ false);
    um.add_files(std::slice::from_ref(&link)).unwrap();
    assert!(um.get_digest_to_file().is_empty());

    let expected_result = ActionResult {
        output_file_symlinks: vec![output_symlink("link", "target")],
        output_symlinks: vec![output_symlink("link", "target")],
        ..Default::default()
    };
    assert_eq!(um.get_action_result(), expected_result);
}

#[test]
fn action_result_relative_directory_symlink_as_symlink() {
    let f = Fixture::set_up();
    let dir = f.exec_root.get_relative_str("dir");
    dir.create_directory().unwrap();
    let file = f.exec_root.get_relative_str("dir/foo");
    write_content(&file, &[1, 2, 3, 4, 5]);
    let link = f.exec_root.get_relative_str("link");
    link.create_symbolic_link_to_fragment(&dir.relative_to(&f.exec_root))
        .unwrap();

    let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ false);
    um.add_files(std::slice::from_ref(&link)).unwrap();
    assert!(um.get_digest_to_file().is_empty());

    let expected_result = ActionResult {
        output_directory_symlinks: vec![output_symlink("link", "dir")],
        output_symlinks: vec![output_symlink("link", "dir")],
        ..Default::default()
    };
    assert_eq!(um.get_action_result(), expected_result);
}

#[test]
fn action_result_no_allow_absolute_symlinks_absolute_dangling_symlink_error() {
    // @TestParameter boolean looping
    for looping in [false, true] {
        let f = Fixture::set_up();
        let link = f.exec_root.get_relative_str("link");
        let target = f.exec_root.get_relative_str("target");
        link.create_symbolic_link(&target).unwrap();
        if looping {
            target.create_symbolic_link(&link).unwrap();
        }

        let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ false);
        let e = expect_io_exception(um.add_files(std::slice::from_ref(&link)));
        let message = e.to_string();
        assert!(message.contains("absolute"), "{message}");
        assert!(
            message.contains(&f.execroot_path_string("link")),
            "{message}"
        );
        assert!(message.contains("target"), "{message}");
    }
}

#[test]
fn action_result_allow_absolute_symlinks_absolute_dangling_symlink_as_symlink() {
    // @TestParameter boolean looping
    for looping in [false, true] {
        let f = Fixture::set_up();
        let link = f.exec_root.get_relative_str("link");
        let target = f.exec_root.get_relative_str("target");
        link.create_symbolic_link(&target).unwrap();
        if looping {
            target.create_symbolic_link(&link).unwrap();
        }

        let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ true);
        um.add_files(std::slice::from_ref(&link)).unwrap();
        assert!(um.get_digest_to_file().is_empty());

        let execroot_target = f.execroot_path_string("target");
        let expected_result = ActionResult {
            output_file_symlinks: vec![output_symlink("link", &execroot_target)],
            output_symlinks: vec![output_symlink("link", &execroot_target)],
            ..Default::default()
        };
        assert_eq!(um.get_action_result(), expected_result);
    }
}

#[test]
fn action_result_relative_dangling_symlink_as_symlink() {
    // @TestParameter boolean looping
    for looping in [false, true] {
        let f = Fixture::set_up();
        let link = f.exec_root.get_relative_str("link");
        let target = f.exec_root.get_relative_str("target");
        link.create_symbolic_link_to_fragment(
            &target.relative_to(&link.get_parent_directory().unwrap()),
        )
        .unwrap();
        if looping {
            target
                .create_symbolic_link_to_fragment(
                    &link.relative_to(&target.get_parent_directory().unwrap()),
                )
                .unwrap();
        }

        let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ false);
        um.add_files(std::slice::from_ref(&link)).unwrap();
        assert!(um.get_digest_to_file().is_empty());

        let expected_result = ActionResult {
            output_file_symlinks: vec![output_symlink("link", "target")],
            output_symlinks: vec![output_symlink("link", "target")],
            ..Default::default()
        };
        assert_eq!(um.get_action_result(), expected_result);
    }
}

#[test]
fn action_result_absolute_file_symlink_in_directory_as_file() {
    let f = Fixture::set_up();
    let dir = f.exec_root.get_relative_str("dir");
    dir.create_directory().unwrap();
    let target = f.exec_root.get_relative_str("target");
    write_content(&target, &[1, 2, 3, 4, 5]);
    let link = f.exec_root.get_relative_str("dir/link");
    link.create_symbolic_link(&target).unwrap();

    let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ false);
    um.add_files(std::slice::from_ref(&dir)).unwrap();
    let digest = f.digest_util.compute_path(&target).unwrap();
    assert_digest_to_file(&um, &[(&digest, &link)]);

    let tree = Tree {
        root: Some(Directory {
            files: vec![file_node("link", &digest, true)],
            ..Default::default()
        }),
        ..Default::default()
    };
    let tree_digest = f.digest_util.compute_message(&tree);

    let expected_result = ActionResult {
        output_directories: vec![output_directory("dir", &tree_digest)],
        ..Default::default()
    };
    assert_eq!(um.get_action_result(), expected_result);
}

#[test]
fn action_result_absolute_directory_symlink_in_directory_as_directory() {
    let f = Fixture::set_up();
    let dir = f.exec_root.get_relative_str("dir");
    dir.create_directory().unwrap();
    let bardir = f.exec_root.get_relative_str("bardir");
    bardir.create_directory().unwrap();
    let foo = f.exec_root.get_relative_str("bardir/foo");
    write_content(&foo, &[1, 2, 3, 4, 5]);
    let link = f.exec_root.get_relative_str("dir/link");
    link.create_symbolic_link(&bardir).unwrap();

    let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ false);
    um.add_files(std::slice::from_ref(&dir)).unwrap();
    let digest = f.digest_util.compute_path(&foo).unwrap();
    assert_digest_to_file(
        &um,
        &[(&digest, &f.exec_root.get_relative_str("dir/link/foo"))],
    );

    let bar_dir = Directory {
        files: vec![file_node("foo", &digest, true)],
        ..Default::default()
    };
    let bar_digest = f.digest_util.compute_message(&bar_dir);
    let tree = Tree {
        root: Some(Directory {
            directories: vec![directory_node("link", &bar_digest)],
            ..Default::default()
        }),
        children: vec![bar_dir],
    };
    let tree_digest = f.digest_util.compute_message(&tree);

    let expected_result = ActionResult {
        output_directories: vec![output_directory("dir", &tree_digest)],
        ..Default::default()
    };
    assert_eq!(um.get_action_result(), expected_result);
}

#[test]
fn action_result_relative_file_symlink_in_directory_as_symlink() {
    let f = Fixture::set_up();
    let dir = f.exec_root.get_relative_str("dir");
    dir.create_directory().unwrap();
    let target = f.exec_root.get_relative_str("target");
    write_content(&target, &[1, 2, 3, 4, 5]);
    let link = f.exec_root.get_relative_str("dir/link");
    link.create_symbolic_link_to_fragment(&PathFragment::create("../target"))
        .unwrap();

    let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ false);
    um.add_files(std::slice::from_ref(&dir)).unwrap();
    assert!(um.get_digest_to_file().is_empty());

    let tree = Tree {
        root: Some(Directory {
            symlinks: vec![symlink_node("link", "../target")],
            ..Default::default()
        }),
        ..Default::default()
    };
    let tree_digest = f.digest_util.compute_message(&tree);

    let expected_result = ActionResult {
        output_directories: vec![output_directory("dir", &tree_digest)],
        ..Default::default()
    };
    assert_eq!(um.get_action_result(), expected_result);
}

#[test]
fn action_result_relative_directory_symlink_in_directory_as_symlink() {
    let f = Fixture::set_up();
    let dir = f.exec_root.get_relative_str("dir");
    dir.create_directory().unwrap();
    let bardir = f.exec_root.get_relative_str("bardir");
    bardir.create_directory().unwrap();
    let foo = f.exec_root.get_relative_str("bardir/foo");
    write_content(&foo, &[1, 2, 3, 4, 5]);
    let link = f.exec_root.get_relative_str("dir/link");
    link.create_symbolic_link_to_fragment(&PathFragment::create("../bardir"))
        .unwrap();

    let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ false);
    um.add_files(std::slice::from_ref(&dir)).unwrap();
    assert!(um.get_digest_to_file().is_empty());

    let tree = Tree {
        root: Some(Directory {
            symlinks: vec![symlink_node("link", "../bardir")],
            ..Default::default()
        }),
        ..Default::default()
    };
    let tree_digest = f.digest_util.compute_message(&tree);

    let expected_result = ActionResult {
        output_directories: vec![output_directory("dir", &tree_digest)],
        ..Default::default()
    };
    assert_eq!(um.get_action_result(), expected_result);
}

#[test]
fn action_result_allow_absolute_symlinks_absolute_dangling_symlink_in_directory_as_symlink() {
    // @TestParameter boolean looping
    for looping in [false, true] {
        let f = Fixture::set_up();
        let dir = f.exec_root.get_relative_str("dir");
        dir.create_directory().unwrap();
        let target = f.exec_root.get_relative_str("target");
        let link = f.exec_root.get_relative_str("dir/link");
        link.create_symbolic_link(&target).unwrap();
        if looping {
            target.create_symbolic_link(&link).unwrap();
        }

        let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ true);
        um.add_files(std::slice::from_ref(&dir)).unwrap();
        assert!(um.get_digest_to_file().is_empty());

        let tree = Tree {
            root: Some(Directory {
                symlinks: vec![symlink_node("link", &f.execroot_path_string("target"))],
                ..Default::default()
            }),
            ..Default::default()
        };
        let tree_digest = f.digest_util.compute_message(&tree);

        let expected_result = ActionResult {
            output_directories: vec![output_directory("dir", &tree_digest)],
            ..Default::default()
        };
        assert_eq!(um.get_action_result(), expected_result);
    }
}

#[test]
fn action_result_no_allow_absolute_symlinks_absolute_dangling_symlink_in_directory_error() {
    // @TestParameter boolean looping
    for looping in [false, true] {
        let f = Fixture::set_up();
        let dir = f.exec_root.get_relative_str("dir");
        dir.create_directory().unwrap();
        let target = f.exec_root.get_relative_str("target");
        let link = f.exec_root.get_relative_str("dir/link");
        link.create_symbolic_link(&target).unwrap();
        if looping {
            target.create_symbolic_link(&link).unwrap();
        }

        let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ false);
        let e = expect_io_exception(um.add_files(std::slice::from_ref(&link)));
        let message = e.to_string();
        assert!(message.contains("absolute"), "{message}");
        assert!(
            message.contains(&f.execroot_path_string("dir/link")),
            "{message}"
        );
        assert!(message.contains("target"), "{message}");
    }
}

#[test]
fn action_result_relative_dangling_symlink_in_directory_as_symlink() {
    // @TestParameter boolean looping
    for looping in [false, true] {
        let f = Fixture::set_up();
        let dir = f.exec_root.get_relative_str("dir");
        dir.create_directory().unwrap();
        let target = f.exec_root.get_relative_str("target");
        let link = f.exec_root.get_relative_str("dir/link");
        // target.relativeTo(link.getParentDirectory()) does not work because relativeTo refuses
        // to create uplevel references.
        link.create_symbolic_link_to_fragment(&PathFragment::create("../target"))
            .unwrap();
        if looping {
            target
                .create_symbolic_link_to_fragment(
                    &link.relative_to(&target.get_parent_directory().unwrap()),
                )
                .unwrap();
        }

        let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ false);
        um.add_files(std::slice::from_ref(&dir)).unwrap();
        assert!(um.get_digest_to_file().is_empty());

        let tree = Tree {
            root: Some(Directory {
                symlinks: vec![symlink_node("link", "../target")],
                ..Default::default()
            }),
            ..Default::default()
        };
        let tree_digest = f.digest_util.compute_message(&tree);

        let expected_result = ActionResult {
            output_directories: vec![output_directory("dir", &tree_digest)],
            ..Default::default()
        };
        assert_eq!(um.get_action_result(), expected_result);
    }
}

#[test]
fn action_result_special_file_error() {
    let f = Fixture::set_up();
    let dir = f.create_directory_with_special_file("dir", "special");
    let special = dir.get_child("special");

    let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ false);
    let message = expect_user_exec_exception(um.add_files(std::slice::from_ref(&special)));
    assert!(message.contains("special file"), "{message}");
    assert!(message.contains("dir/special"), "{message}");
}

#[test]
fn action_result_special_file_in_directory_error() {
    let f = Fixture::set_up();
    let dir = f.create_directory_with_special_file("dir", "special");

    let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ false);
    let message = expect_user_exec_exception(um.add_files(std::slice::from_ref(&dir)));
    assert!(message.contains("special file"), "{message}");
    assert!(message.contains("dir/special"), "{message}");
}

#[test]
fn action_result_follow_symlinks_absolute_symlink_to_special_file_error() {
    let f = Fixture::set_up();
    let dir = f.create_directory_with_symlink_to_special_file("dir", "link", "special");
    let link = dir.get_child("link");

    let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ false);
    let message = expect_user_exec_exception(um.add_files(std::slice::from_ref(&link)));
    assert!(message.contains("special file"), "{message}");
    assert!(message.contains("dir/link"), "{message}");
}

#[test]
fn action_result_follow_symlinks_absolute_symlink_to_special_file_in_directory_error() {
    let f = Fixture::set_up();
    let dir = f.create_directory_with_symlink_to_special_file("dir", "link", "special");

    let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ false);
    let message = expect_user_exec_exception(um.add_files(std::slice::from_ref(&dir)));
    assert!(message.contains("special file"), "{message}");
    assert!(message.contains("dir/link"), "{message}");
}

#[test]
fn action_result_topologically_sorted_and_deduplicated_tree() {
    // Create 5^3 identical files named "dir/%d/%d/%d/file".
    let f = Fixture::set_up();
    let dir = f.exec_root.get_relative_str("dir");
    dir.create_directory().unwrap();
    let file_contents: [u8; 5] = [1, 2, 3, 4, 5];
    let children_per_directory = 5;
    for a in 0..children_per_directory {
        let path_a = dir.get_relative_str(&a.to_string());
        path_a.create_directory().unwrap();
        for b in 0..children_per_directory {
            let path_b = path_a.get_relative_str(&b.to_string());
            path_b.create_directory().unwrap();
            for c in 0..children_per_directory {
                let path_c = path_b.get_relative_str(&c.to_string());
                path_c.create_directory().unwrap();
                let file = path_c.get_relative_str("file");
                write_content(&file, &file_contents);
            }
        }
    }

    let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ false);
    um.add_files(std::slice::from_ref(&dir)).unwrap();

    // Even though we constructed 1 + 5 + 5^2 + 5^3 directories, the resulting
    // Tree message should only contain four unique instances. The directories
    // should also be topologically sorted.
    let mut children: Vec<Directory> = Vec::new();
    let mut root = Directory {
        files: vec![file_node(
            "file",
            &f.digest_util.compute(&file_contents),
            true,
        )],
        ..Default::default()
    };
    for _depth in 0..3 {
        let mut b = Directory::default();
        let parent_digest = f.digest_util.compute_message(&root);
        for i in 0..children_per_directory {
            b.directories
                .push(directory_node(&i.to_string(), &parent_digest));
        }
        children.insert(0, root);
        root = b;
    }
    let tree = Tree {
        root: Some(root),
        children,
    };
    let tree_digest = f.digest_util.compute_message(&tree);

    let expected_result = ActionResult {
        output_directories: vec![output_directory("dir", &tree_digest)],
        ..Default::default()
    };
    assert_eq!(um.get_action_result(), expected_result);
}

#[test]
fn action_result_preserve_executable_bit_executable_file() {
    let f = Fixture::set_up();
    let file = f.exec_root.get_relative_str("file");
    write_content(&file, &[1, 2, 3, 4, 5]);
    file.set_executable(true).unwrap();

    let mut um = f.new_upload_manifest_with_preserve_executable_bit(
        /* allowAbsoluteSymlinks= */ false, /* preserveExecutableBit= */ true,
    );
    um.add_files(std::slice::from_ref(&file)).unwrap();
    let digest = f.digest_util.compute_path(&file).unwrap();
    assert_digest_to_file(&um, &[(&digest, &file)]);

    let expected_result = ActionResult {
        output_files: vec![output_file("file", &digest, true)],
        ..Default::default()
    };
    assert_eq!(um.get_action_result(), expected_result);
}

#[test]
fn action_result_preserve_executable_bit_non_executable_file() {
    let f = Fixture::set_up();
    let file = f.exec_root.get_relative_str("file");
    write_content(&file, &[1, 2, 3, 4, 5]);
    file.set_executable(false).unwrap();

    let mut um = f.new_upload_manifest_with_preserve_executable_bit(
        /* allowAbsoluteSymlinks= */ false, /* preserveExecutableBit= */ true,
    );
    um.add_files(std::slice::from_ref(&file)).unwrap();
    let digest = f.digest_util.compute_path(&file).unwrap();
    assert_digest_to_file(&um, &[(&digest, &file)]);

    let expected_result = ActionResult {
        output_files: vec![output_file("file", &digest, false)],
        ..Default::default()
    };
    assert_eq!(um.get_action_result(), expected_result);
}

#[test]
fn action_result_preserve_executable_bit_mixed_files_in_directory() {
    let f = Fixture::set_up();
    let dir = f.exec_root.get_relative_str("dir");
    dir.create_directory().unwrap();
    let executable_file = f.exec_root.get_relative_str("dir/executable");
    write_content(&executable_file, &[1, 2, 3]);
    executable_file.set_executable(true).unwrap();
    let non_executable_file = f.exec_root.get_relative_str("dir/nonexecutable");
    write_content(&non_executable_file, &[4, 5, 6]);
    non_executable_file.set_executable(false).unwrap();

    let mut um = f.new_upload_manifest_with_preserve_executable_bit(
        /* allowAbsoluteSymlinks= */ false, /* preserveExecutableBit= */ true,
    );
    um.add_files(std::slice::from_ref(&dir)).unwrap();

    let executable_digest = f.digest_util.compute_path(&executable_file).unwrap();
    let non_executable_digest = f.digest_util.compute_path(&non_executable_file).unwrap();
    assert_digest_to_file(
        &um,
        &[
            (&executable_digest, &executable_file),
            (&non_executable_digest, &non_executable_file),
        ],
    );

    let tree = Tree {
        root: Some(Directory {
            files: vec![
                file_node("executable", &executable_digest, true),
                file_node("nonexecutable", &non_executable_digest, false),
            ],
            ..Default::default()
        }),
        ..Default::default()
    };
    let tree_digest = f.digest_util.compute_message(&tree);

    let expected_result = ActionResult {
        output_directories: vec![output_directory("dir", &tree_digest)],
        ..Default::default()
    };
    assert_eq!(um.get_action_result(), expected_result);
}

#[test]
fn action_result_unicode_directory_tree() {
    // Verify that non-ASCII names in directory trees are converted from Bazel's internal
    // encoding to Unicode in the protobuf messages (FileNode, DirectoryNode, SymlinkNode).
    let f = Fixture::set_up();
    let dir = f.exec_root.get_relative_str(&unicode_to_internal("dïr"));
    dir.create_directory().unwrap();
    let subdir = f
        .exec_root
        .get_relative_str(&unicode_to_internal("dïr/sübdïr"));
    subdir.create_directory().unwrap();
    let file = f
        .exec_root
        .get_relative_str(&unicode_to_internal("dïr/sübdïr/fïlé"));
    write_content(&file, &[1, 2, 3, 4, 5]);
    let link = f
        .exec_root
        .get_relative_str(&unicode_to_internal("dïr/lïnk"));
    link.create_symbolic_link_to_fragment(&PathFragment::create(&unicode_to_internal("../tàrgét")))
        .unwrap();

    let mut um = f.new_upload_manifest(/* allowAbsoluteSymlinks= */ false);
    um.add_files(std::slice::from_ref(&dir)).unwrap();
    let file_digest = f.digest_util.compute_path(&file).unwrap();

    // Build the expected tree with Unicode names.
    let subdir_dir = Directory {
        files: vec![file_node("fïlé", &file_digest, true)],
        ..Default::default()
    };
    let subdir_digest = f.digest_util.compute_message(&subdir_dir);
    let root_dir = Directory {
        directories: vec![directory_node("sübdïr", &subdir_digest)],
        symlinks: vec![symlink_node("lïnk", "../tàrgét")],
        ..Default::default()
    };
    let tree = Tree {
        root: Some(root_dir),
        children: vec![subdir_dir],
    };
    let tree_digest = f.digest_util.compute_message(&tree);

    let expected_result = ActionResult {
        output_directories: vec![output_directory("dïr", &tree_digest)],
        ..Default::default()
    };
    assert_eq!(um.get_action_result(), expected_result);
}
