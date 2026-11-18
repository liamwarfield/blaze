//! Smoke tests for `UnixFileSystem`, loosely derived from `FileSystemTest.java` (which is not
//! ported in full).

use std::io::{Read, Write};

use blaze_vfs::{
    DigestHashFunction, DirentType, FileSystemExt, FileSystemRef, NotASymlinkException, Path,
    PathFragment, Symlinks, UnixFileSystem,
};

struct Fixture {
    work: Path,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let fs: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
        let tmp = std::env::temp_dir();
        let work = fs
            .get_path(tmp.to_str().expect("utf-8 temp dir"))
            .get_relative_str(&format!(
                "blaze_vfs_fs_test_{}_{}",
                std::process::id(),
                name
            ));
        let _ = work.delete_tree();
        work.create_directory_and_parents().expect("mkdir");
        Fixture { work }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.work.delete_tree();
    }
}

fn write(p: &Path, content: &str) {
    let mut out = p.get_output_stream().expect("open");
    out.write_all(content.as_bytes()).expect("write");
}

fn read(p: &Path) -> String {
    let mut s = String::new();
    p.get_input_stream()
        .expect("open")
        .read_to_string(&mut s)
        .expect("read");
    s
}

#[test]
fn create_directory_returns_false_if_exists() {
    let f = Fixture::new("mkdir");
    let d = f.work.get_child("d");
    assert!(d.create_directory().unwrap());
    assert!(!d.create_directory().unwrap());
    let file = f.work.get_child("file");
    write(&file, "x");
    let err = file.create_directory().unwrap_err();
    assert_eq!(err.to_string(), format!("{} (File exists)", file));
}

#[test]
fn write_read_stat_and_digest() {
    let f = Fixture::new("rw");
    let file = f.work.get_child("a.txt");
    write(&file, "hello");
    assert_eq!(read(&file), "hello");
    assert!(file.is_file());
    assert!(!file.is_directory());
    assert_eq!(file.get_file_size().unwrap(), 5);
    assert_eq!(
        file.get_digest().unwrap(),
        [
            0x2c, 0xf2, 0x4d, 0xba, 0x5f, 0xb0, 0xa3, 0x0e, 0x26, 0xe8, 0x3b, 0x2a, 0xc5, 0xb9,
            0xe2, 0x9e, 0x1b, 0x16, 0x1e, 0x5c, 0x1f, 0xa7, 0x42, 0x5e, 0x73, 0x04, 0x33, 0x62,
            0x93, 0x8b, 0x98, 0x24
        ]
    );
    let mut append = file.get_output_stream_with(true, false).unwrap();
    append.write_all(b"!").unwrap();
    drop(append);
    assert_eq!(read(&file), "hello!");
}

#[test]
fn missing_file_errors_follow_native_format() {
    let f = Fixture::new("missing");
    let missing = f.work.get_child("missing");
    let err = missing.stat().unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(
        err.to_string(),
        format!("{} (No such file or directory)", missing)
    );
    assert!(missing.stat_if_found(Symlinks::Follow).unwrap().is_none());
    assert!(!missing.delete().unwrap());
}

#[test]
fn symlinks_and_readdir() {
    let f = Fixture::new("symlinks");
    let dir = f.work.get_child("dir");
    dir.create_directory().unwrap();
    let file = dir.get_child("file");
    write(&file, "x");
    let link = f.work.get_child("link");
    link.create_symbolic_link_to_fragment(&PathFragment::create("dir"))
        .unwrap();
    assert!(link.is_symbolic_link());
    assert_eq!(
        link.read_symbolic_link().unwrap(),
        PathFragment::create("dir")
    );
    assert_eq!(link.resolve_symbolic_links().unwrap(), dir);
    assert_eq!(
        link.get_child("file").resolve_symbolic_links().unwrap(),
        file
    );
    let err = file.read_symbolic_link().unwrap_err();
    assert!(NotASymlinkException::is(&err));
    assert_eq!(err.to_string(), format!("{} is not a symlink", file));

    let mut entries = f.work.readdir(Symlinks::Nofollow).unwrap();
    entries.sort();
    let shown: Vec<String> = entries.iter().map(|d| d.to_string()).collect();
    assert_eq!(shown, vec!["dir[directory]", "link[symlink]"]);
    let followed = f.work.readdir(Symlinks::Follow).unwrap();
    assert!(
        followed
            .iter()
            .all(|d| d.get_type() == DirentType::Directory)
    );
}

#[test]
fn symlink_loop_is_detected() {
    let f = Fixture::new("loop");
    let a = f.work.get_child("a");
    let b = f.work.get_child("b");
    a.create_symbolic_link(&b).unwrap();
    b.create_symbolic_link(&a).unwrap();
    let err = a.resolve_symbolic_links().unwrap_err();
    assert!(
        err.to_string()
            .ends_with("(Too many levels of symbolic links)")
    );
}

#[test]
fn delete_tree_and_permissions() {
    let f = Fixture::new("delete");
    let d = f.work.get_relative_str("x/y/z");
    d.create_directory_and_parents().unwrap();
    let file = d.get_child("f");
    write(&file, "x");
    file.set_executable(true).unwrap();
    assert!(file.is_executable().unwrap());
    file.chmod(0o400).unwrap();
    assert!(!file.is_writable().unwrap());
    // A read-only directory still gets deleted.
    d.set_writable(false).unwrap();
    f.work.get_child("x").delete_tree().unwrap();
    assert!(!f.work.get_child("x").exists());
}

#[test]
fn modification_time_and_rename() {
    let f = Fixture::new("mtime");
    let file = f.work.get_child("f");
    write(&file, "x");
    file.set_last_modified_time(42_000).unwrap();
    assert_eq!(file.get_last_modified_time().unwrap(), 42_000);
    let target = f.work.get_child("g");
    file.rename_to(&target).unwrap();
    assert!(!file.exists());
    assert!(target.exists());
    let link = f.work.get_child("h");
    target.create_hard_link(&link).unwrap();
    assert_eq!(read(&link), "x");
    let tmp = f.work.create_temp_directory("tmp").unwrap();
    assert!(tmp.is_directory());
    assert!(tmp.get_base_name().starts_with("tmp"));
}
