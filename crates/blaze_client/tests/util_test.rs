//! Ports of src/test/cpp/util/md5_test.cc, numbers_test.cc, path_posix_test.cc, file_test.cc and
//! file_posix_test.cc (the parts for functions the client port has).

use std::os::unix::fs::PermissionsExt;
use std::sync::Mutex;

use blaze_client::util::errors::errno;
use blaze_client::util::file::*;
use blaze_client::util::md5::Md5Digest;
use blaze_client::util::numbers::safe_strto32;
use blaze_client::util::path::*;

/// Serializes the tests that change or read the process cwd and environment.
static PROCESS_STATE: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    PROCESS_STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn tmpdir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn s(p: &std::path::Path) -> String {
    p.to_str().unwrap().to_string()
}

// Java: src/test/cpp/util/md5_test.cc:21 BlazeUtil.Basic
#[test]
fn basic() {
    let strs = [
        "",
        "a",
        "abc",
        "message digest",
        "abcdefghijklmnopqrstuvwxyz",
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
        "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
    ];
    let md5s = [
        "d41d8cd98f00b204e9800998ecf8427e",
        "0cc175b9c0f1b6a831c399e269772661",
        "900150983cd24fb0d6963f7d28e17f72",
        "f96b697d7cb7938d525a2f31aaf161d0",
        "c3fcd3d76192e4007dfb496cca67e13b",
        "d174ab98d277d9f5a5611c2c9f419d9f",
        "57edf4a22be3c955ac49da2e2107b67a",
    ];
    assert_eq!(strs.len(), md5s.len());
    let mut digest = Md5Digest::new();
    for i in 0..strs.len() {
        digest.reset();
        digest.update(strs[i].as_bytes());
        digest.finish();
        assert_eq!(md5s[i], digest.string());
    }
}

// Java: src/test/cpp/util/numbers_test.cc:20 NumbersTest.TestSafeStrto32
#[test]
fn test_safe_strto32() {
    assert_eq!((true, 0), safe_strto32("0"));
    assert_eq!((true, 42), safe_strto32("42"));
    assert_eq!((true, 7), safe_strto32("007"));
    assert_eq!((true, 1234567), safe_strto32("1234567"));
    assert_eq!((true, 0), safe_strto32("-0"));
    assert_eq!((true, -273), safe_strto32("-273"));
    assert_eq!((true, -420), safe_strto32("-0420"));
}

// Java: src/test/cpp/util/path_posix_test.cc:30 PathPosixTest.TestDirname
#[test]
fn test_dirname() {
    assert_eq!("", dirname(""));
    assert_eq!("/", dirname("/"));
    assert_eq!("", dirname("foo"));
    assert_eq!("/", dirname("/foo"));
    assert_eq!("/foo", dirname("/foo/"));
    assert_eq!("foo", dirname("foo/bar"));
    assert_eq!("foo/bar", dirname("foo/bar/baz"));
    assert_eq!("", dirname("\\foo"));
    assert_eq!("", dirname("\\foo\\"));
    assert_eq!("", dirname("foo\\bar"));
    assert_eq!("", dirname("foo\\bar\\baz"));
    assert_eq!("foo\\bar", dirname("foo\\bar/baz\\qux"));
    assert_eq!("c:", dirname("c:/"));
    assert_eq!("", dirname("c:\\"));
    assert_eq!("c:", dirname("c:/foo"));
    assert_eq!("", dirname("c:\\foo"));
    assert_eq!("", dirname("\\\\?\\c:\\"));
    assert_eq!("", dirname("\\\\?\\c:\\foo"));
}

// Java: src/test/cpp/util/path_posix_test.cc:53 PathPosixTest.TestBasename
#[test]
fn test_basename() {
    assert_eq!("", basename(""));
    assert_eq!("", basename("/"));
    assert_eq!("foo", basename("foo"));
    assert_eq!("foo", basename("/foo"));
    assert_eq!("", basename("/foo/"));
    assert_eq!("bar", basename("foo/bar"));
    assert_eq!("baz", basename("foo/bar/baz"));
    assert_eq!("\\foo", basename("\\foo"));
    assert_eq!("\\foo\\", basename("\\foo\\"));
    assert_eq!("foo\\bar", basename("foo\\bar"));
    assert_eq!("foo\\bar\\baz", basename("foo\\bar\\baz"));
    assert_eq!("baz\\qux", basename("foo\\bar/baz\\qux"));
    assert_eq!("qux", basename("qux"));
    assert_eq!("", basename("c:/"));
    assert_eq!("c:\\", basename("c:\\"));
    assert_eq!("foo", basename("c:/foo"));
    assert_eq!("c:\\foo", basename("c:\\foo"));
    assert_eq!("\\\\?\\c:\\", basename("\\\\?\\c:\\"));
    assert_eq!("\\\\?\\c:\\foo", basename("\\\\?\\c:\\foo"));
}

// Java: src/test/cpp/util/path_posix_test.cc:77 PathPosixTest.JoinPath
#[test]
fn join_path_test() {
    assert_eq!("", join_path("", ""));
    assert_eq!("a/b", join_path("a", "b"));
    assert_eq!("a/b", join_path("a/", "b"));
    assert_eq!("a/b", join_path("a", "/b"));
    assert_eq!("a/b", join_path("a/", "/b"));
    assert_eq!("/", join_path("/", "/"));
}

// Java: src/test/cpp/util/path_posix_test.cc:97 PathPosixTest.GetCwd
#[test]
fn get_cwd_test() {
    let _g = lock();
    let cwd = s(&std::env::current_dir().unwrap());
    // Assert that GetCwd() and getcwd() return the same value.
    assert_eq!(cwd, get_cwd());
    // Change to a different directory.
    assert!(change_directory("/usr"));
    // Assert that GetCwd() returns the new CWD.
    assert_eq!("/usr", get_cwd());
    assert!(change_directory(&cwd));
    assert_eq!(cwd, get_cwd());
}

// Java: src/test/cpp/util/path_posix_test.cc:115 PathPosixTest.IsAbsolute
#[test]
fn is_absolute_test() {
    assert!(!is_absolute(""));
    assert!(is_absolute("/"));
    assert!(is_absolute("/foo"));
    assert!(!is_absolute("\\"));
    assert!(!is_absolute("\\foo"));
    assert!(!is_absolute("c:"));
    assert!(!is_absolute("c:/"));
    assert!(!is_absolute("c:\\"));
    assert!(!is_absolute("c:\\foo"));
    assert!(!is_absolute("\\\\?\\c:\\"));
    assert!(!is_absolute("\\\\?\\c:\\foo"));
}

// Java: src/test/cpp/util/path_posix_test.cc:129 PathPosixTest.IsRootDirectory
#[test]
fn is_root_directory_test() {
    assert!(!is_root_directory(""));
    assert!(is_root_directory("/"));
    assert!(!is_root_directory("/foo"));
    assert!(!is_root_directory("\\"));
    assert!(!is_root_directory("\\foo"));
    assert!(!is_root_directory("c:"));
    assert!(!is_root_directory("c:/"));
    assert!(!is_root_directory("c:\\"));
    assert!(!is_root_directory("c:\\foo"));
    assert!(!is_root_directory("\\\\?\\c:\\"));
    assert!(!is_root_directory("\\\\?\\c:\\foo"));
}

// Java: src/test/cpp/util/path_posix_test.cc:143 PathPosixTest.IsDevNullTest
// Java: src/test/cpp/util/file_test.cc:310 FileTest.IsDevNullTest
#[test]
fn is_dev_null_test() {
    assert!(is_dev_null(Some("/dev/null")));
    assert!(!is_dev_null(Some("dev/null")));
    assert!(!is_dev_null(Some("/dev/nul")));
    assert!(!is_dev_null(Some("/dev/nulll")));
    assert!(!is_dev_null(None));
    assert!(!is_dev_null(Some("")));
}

// Java: src/test/cpp/util/path_posix_test.cc:152 PathPosixTest.MakeAbsolute
#[test]
fn make_absolute_test() {
    let _g = lock();
    assert_eq!(make_absolute("/foo/bar"), "/foo/bar");
    assert_eq!(make_absolute("/foo/bar/"), "/foo/bar/");
    assert_eq!(make_absolute("foo"), get_cwd() + "/foo");
    assert_eq!(make_absolute("/dev/null"), "/dev/null");
    assert_eq!(make_absolute(""), "");
}

// Java: src/test/cpp/util/path_posix_test.cc:162 PathPosixTest.MakeAbsoluteAndResolveEnvvars
#[test]
fn make_absolute_and_resolve_envvars_test() {
    let _g = lock();
    let tmp = tmpdir();
    let tmpdir = s(tmp.path());
    // SAFETY: PROCESS_STATE serializes environment access in this test binary.
    unsafe {
        blaze_client::platform::set_env("TEST_TMPDIR", &tmpdir);
        blaze_client::platform::set_env("PATH_POSIX_TEST_ENV", "${TEST_TMPDIR}");
    }
    let expected_tmpdir_bar = convert_path(&(tmpdir.clone() + "/bar"));

    // Using an existing environment variable
    assert_eq!(
        expected_tmpdir_bar,
        make_absolute_and_resolve_envvars("${TEST_TMPDIR}/bar")
    );
    // Using an undefined environment variable (case-sensitive)
    assert_eq!(
        "/bar",
        make_absolute_and_resolve_envvars("${test_tmpdir}/bar")
    );
    // This style of variable is not supported
    assert_eq!(
        join_path(&get_cwd(), "$TEST_TMPDIR/bar"),
        make_absolute_and_resolve_envvars("$TEST_TMPDIR/bar")
    );
    // Only one layer of variables is expanded, we do not recurse
    assert_eq!(
        join_path(&get_cwd(), "${TEST_TMPDIR}/bar"),
        make_absolute_and_resolve_envvars("${PATH_POSIX_TEST_ENV}/bar")
    );
    // Check that Windows-style envvars are not resolved when not on Windows.
    assert_eq!(
        make_absolute_and_resolve_envvars("%PATH%"),
        join_path(&get_cwd(), "%PATH%")
    );
    // SAFETY: as above.
    unsafe {
        blaze_client::platform::unset_env("TEST_TMPDIR");
        blaze_client::platform::unset_env("PATH_POSIX_TEST_ENV");
    }
}

// Java: src/test/cpp/util/path_posix_test.cc:187 PathPosixTest.NormalizeAbsPath
#[test]
fn normalize_abs_path() {
    let n = test_only_normalize_abs_path;
    assert_eq!(n(""), "");
    assert_eq!(n("not_absolute"), "");
    assert_eq!(n("../x/y"), "");
    assert_eq!(n("./"), "");

    assert_eq!(n("/."), "/");
    assert_eq!(n("/./"), "/");
    assert_eq!(n("//.//"), "/");
    assert_eq!(n("/."), "/");
    assert_eq!(n("//."), "/");

    assert_eq!(n("/../"), "/");
    assert_eq!(n("//..//"), "/");
    assert_eq!(n("/.."), "/");
    assert_eq!(n("//.."), "/");

    assert_eq!(n("/x"), "/x");
    assert_eq!(n("/x/"), "/x/");
    assert_eq!(n("//x//"), "/x/");

    assert_eq!(n("/x/.."), "/");
    assert_eq!(n("/x/../"), "/");
    assert_eq!(n("//x//..//"), "/");
    assert_eq!(n("//x//y//"), "/x/y/");

    assert_eq!(n("/x/./y/"), "/x/y/");
    assert_eq!(n("/x/../y/"), "/y/");
    assert_eq!(n("/x/../../y/"), "/y/");
    assert_eq!(n("/x/../y/.."), "/");
    assert_eq!(n("/x/../y/../"), "/");
    assert_eq!(n("/./x/../y/../"), "/");

    assert_eq!(n("/foo"), "/foo");
    assert_eq!(n("/foo/"), "/foo/");
    assert_eq!(n("//foo//"), "/foo/");

    assert_eq!(n("/foo/.."), "/");
    assert_eq!(n("/foo/../"), "/");
    assert_eq!(n("//foo//..//"), "/");
    assert_eq!(n("//foo//bar//"), "/foo/bar/");

    assert_eq!(n("/foo/./bar/"), "/foo/bar/");
    assert_eq!(n("/foo/../bar/"), "/bar/");
    assert_eq!(n("/foo/../../bar/"), "/bar/");
    assert_eq!(n("/foo/../bar/.."), "/");
    assert_eq!(n("/foo/../bar/../"), "/");
    assert_eq!(n("/./foo/../bar/../"), "/");
}

// Java: src/test/cpp/util/file_test.cc:71 FileTest.TestReadFileIntoString
#[test]
fn test_read_file_into_string() {
    let tmp = tmpdir();
    let filename = join_path(&s(tmp.path()), "test.readfile");
    std::fs::write(&filename, "hello world").unwrap();
    assert_eq!(read_file(&filename, 0).unwrap(), b"hello world");
    assert_eq!(read_file(&filename, 5).unwrap(), b"hello");
    assert_eq!(read_file("/dev/null", 42).unwrap(), b"");
}

// Java: src/test/cpp/util/file_test.cc:93 FileTest.TestReadFileIntoBuffer
#[test]
#[ignore = "ReadFile(path, void* data, size_t size) is not ported: the client does not use it"]
fn test_read_file_into_buffer() {}

// Java: src/test/cpp/util/file_test.cc:118 FileTest.TestWriteFile
#[test]
fn test_write_file() {
    let tmp = tmpdir();
    let filename = join_path(&s(tmp.path()), "test.writefile");
    assert!(write_file(&b"hello"[..3], &filename, 0o644));
    assert_eq!(std::fs::read(&filename).unwrap(), b"hel");
    assert!(write_file(b"hello", &filename, 0o644));
    assert_eq!(std::fs::read(&filename).unwrap(), b"hello");
    // PORT: the C++ also writes "/dev/null"; WriteFile unlinks its target first, which would
    // delete /dev/null when the tests run as root, so that step is left out.
    std::fs::remove_file(&filename).unwrap();
}

// Java: src/test/cpp/util/file_test.cc:147 FileTest.TestLargeFileWrite
#[test]
#[ignore = "allocates 4 GB, and WriteFile unlinks /dev/null first when run as root; run manually as non-root"]
fn test_large_file_write() {
    let buffer = vec![0u8; 4_000_000_000];
    assert!(write_file(&buffer, "/dev/null", 0o644));
}

// Java: src/test/cpp/util/file_test.cc:156 FileTest.TestMtimeHandling
#[test]
fn test_mtime_handling() {
    let tmp = tmpdir();
    let tempdir = Path::new(&s(tmp.path()));
    // Assert that a directory is always untampered with. (We do not care about directories'
    // mtimes.)
    assert!(is_untampered(&tempdir));
    // Create a new file, assert its mtime is not in the future.
    let file = tempdir.get_relative("foo.txt");
    assert!(write_file_path(b"hello", &file, 0o644));
    assert!(!is_untampered(&file));
    // Set the file's mtime to the future, assert that it's so.
    assert!(set_mtime_to_distant_future(&file));
    assert!(is_untampered(&file));
    // Overwrite the file, resetting its mtime, assert that IsUntampered notices.
    assert!(write_file_path(b"world", &file, 0o644));
    assert!(!is_untampered(&file));
    // Set it to the future again so we can reset it using SetToNow.
    assert!(set_mtime_to_distant_future(&file));
    assert!(is_untampered(&file));
    // Assert that SetToNow resets the timestamp.
    assert!(set_mtime_to_now(&file));
    assert!(!is_untampered(&file));
    // Delete the file and assert that we can no longer set or query its mtime.
    assert!(unlink_path_path(&file));
    assert!(!set_mtime_to_now(&file));
    assert!(!set_mtime_to_distant_future(&file));
    assert!(!is_untampered(&file));
}

// Java: src/test/cpp/util/file_test.cc:189 FileTest.TestCreateSiblingTempDir
#[test]
fn test_create_sibling_temp_dir() {
    let tmp = tmpdir();
    let tempdir = Path::new(&s(tmp.path()));

    let input_in_existing = tempdir.get_relative("other");
    let output_in_existing = create_sibling_temp_dir(&input_in_existing);
    assert_ne!(input_in_existing, output_in_existing);
    assert_eq!(
        input_in_existing.get_parent(),
        output_in_existing.get_parent()
    );
    assert!(
        output_in_existing
            .get_base_name()
            .starts_with(&(input_in_existing.get_base_name() + ".tmp."))
    );
    assert!(path_exists_path(&output_in_existing));

    let missing_dir = tempdir.get_relative("doesntexistyet");
    assert!(!path_exists_path(&missing_dir));
    let input_in_missing = missing_dir.get_relative("other");
    let output_in_missing = create_sibling_temp_dir(&input_in_missing);
    assert_ne!(input_in_missing, output_in_missing);
    assert_eq!(
        input_in_missing.get_parent(),
        output_in_missing.get_parent()
    );
    assert!(
        output_in_missing
            .get_base_name()
            .starts_with(&(input_in_missing.get_base_name() + ".tmp."))
    );
    assert!(path_exists_path(&output_in_missing));
}

// Java: src/test/cpp/util/file_test.cc:214 FileTest.TestRenameDirectory
#[test]
fn test_rename_directory() {
    let tmp = tmpdir();
    let tempdir = Path::new(&s(tmp.path()));

    let dir1 = tempdir.get_relative("test_rename_dir/dir1");
    let dir2 = tempdir.get_relative("test_rename_dir/dir2");
    assert!(make_directories_path(&dir1, 0o700));
    let file1 = dir1.get_relative("file1.txt");
    assert!(write_file_path(b"hello", &file1, 0o644));

    assert_eq!(rename_directory(&dir1, &dir2), K_RENAME_DIRECTORY_SUCCESS);
    assert_eq!(
        rename_directory(&dir1, &dir2),
        K_RENAME_DIRECTORY_FAILURE_OTHER_ERROR
    );
    assert!(make_directories_path(&dir1, 0o700));
    assert!(write_file_path(b"hello", &file1, 0o644));
    assert_eq!(
        rename_directory(&dir2, &dir1),
        K_RENAME_DIRECTORY_FAILURE_NOT_EMPTY
    );
}

// Java: src/test/cpp/util/file_test.cc:274 FilePosixTest.GetAllFilesUnder
#[test]
fn get_all_files_under_test() {
    let tmp = tmpdir();
    let tmpdir = Path::new(&s(tmp.path()));

    let root = tmpdir.get_relative("FilePosixTest.GetAllFilesUnder.root");
    let file1 = root.get_relative("file1");
    let dir1 = root.get_relative("dir1");
    let dir2 = root.get_relative("dir2");
    let dir3 = dir1.get_relative("dir3");
    let file2 = dir1.get_relative("file2");
    let file3 = dir2.get_relative("file3");
    let file4 = dir3.get_relative("file4");
    let file5 = dir3.get_relative("file5");

    for d in [&root, &dir1, &dir2, &dir3] {
        assert!(make_directories_path(d, 0o700));
    }
    for f in [&file1, &file2, &file3, &file4, &file5] {
        assert!(write_file_path(b"", f, 0o644));
    }

    let mut result = Vec::new();
    get_all_files_under(&root, &mut result);
    result.sort();

    let expected = vec![file4, file5, file2, file3, file1];
    assert_eq!(expected, result);

    assert!(remove_recursively(&root));
}

// Java: src/test/cpp/util/file_test.cc:319 FileTest.TestRemoveRecursively
#[test]
fn test_remove_recursively() {
    let tmp = tmpdir();
    let tempdir = Path::new(&s(tmp.path()));
    assert!(path_exists_path(&tempdir));

    let non_existent_dir = tempdir.get_relative("test_rmr_non_existent");
    assert!(remove_recursively(&non_existent_dir));
    assert!(!path_exists_path(&non_existent_dir));

    let empty_dir = tempdir.get_relative("test_rmr_empty_dir");
    assert!(make_directories_path(&empty_dir, 0o700));
    assert!(remove_recursively(&empty_dir));
    assert!(!path_exists_path(&empty_dir));

    let dir_with_content = tempdir.get_relative("test_rmr_dir_w_content");
    assert!(make_directories_path(&dir_with_content, 0o700));
    assert!(write_file_path(
        b"junkdata",
        &dir_with_content.get_relative("file"),
        0o644
    ));
    let subdir = dir_with_content.get_relative("dir");
    assert!(make_directories_path(&subdir, 0o700));
    let subsubdir = subdir.get_relative("dir");
    assert!(make_directories_path(&subsubdir, 0o700));
    assert!(write_file_path(
        b"junkdata",
        &subsubdir.get_relative("deep_file"),
        0o644
    ));
    assert!(remove_recursively(&dir_with_content));
    assert!(!path_exists_path(&dir_with_content));

    let regular_file = tempdir.get_relative("test_rmr_regular_file");
    assert!(write_file_path(b"junkdata", &regular_file, 0o644));
    assert!(remove_recursively(&regular_file));
    assert!(!path_exists_path(&regular_file));
}

// Java: src/test/cpp/util/file_test.cc:36 FileTest.TestSingleThreadedPipe
#[test]
#[ignore = "IPipe is not ported: there is no cancel thread in-process"]
fn test_single_threaded_pipe() {}

// Java: src/test/cpp/util/file_test.cc:50 FileTest.TestMultiThreadedPipe
#[test]
#[ignore = "IPipe is not ported: there is no cancel thread in-process"]
fn test_multi_threaded_pipe() {}

// Java: src/test/cpp/util/file_test.cc:242 FileTest.ForEachDirectoryEntryTest
#[test]
fn for_each_directory_entry_test() {
    let tmp = tmpdir();
    let tmpdir = Path::new(&s(tmp.path()));
    assert!(path_exists_path(&tmpdir));
    let rootdir = tmpdir.get_relative("foo");
    let file1 = rootdir.get_relative("file1.txt");
    let file2 = rootdir.get_relative("file2.txt");
    let subdir = rootdir.get_relative("bar");
    let file3 = subdir.get_relative("file3.txt");

    assert!(make_directories_path(&subdir, 0o700));
    assert!(write_file_path(b"hello", &file1, 0o644));
    assert!(write_file_path(b"hello", &file2, 0o644));
    assert!(write_file_path(b"hello", &file3, 0o644));

    let mut expected = std::collections::BTreeMap::new();
    expected.insert(file1, false);
    expected.insert(file2, false);
    expected.insert(subdir, true);

    let mut entries = std::collections::BTreeMap::new();
    blaze_client::util::file::for_each_directory_entry(&rootdir, &mut |p, is_dir| {
        entries.insert(p.clone(), is_dir);
    });
    assert_eq!(entries, expected);
}

// Java: src/test/cpp/util/file_posix_test.cc:213 FilePosixTest.ForEachDirectoryEntry
#[test]
fn for_each_directory_entry() {
    let tmp = tmpdir();
    let tmpdir = Path::new(&s(tmp.path()));
    assert!(path_exists_path(&tmpdir));

    // Names of mock files and directories.
    let root = tmpdir.get_relative("FilePosixTest.ForEachDirectoryEntry.root");
    let dir = root.get_relative("dir");
    let file = root.get_relative("file");
    let dir_sym = root.get_relative("dir_sym");
    let file_sym = root.get_relative("file_sym");
    let subfile = dir.get_relative("subfile");
    let subfile_through_sym = dir_sym.get_relative("subfile");

    // Create mock directory, file, and symlinks.
    assert!(make_directories_path(&root, 0o700));
    assert!(write_file_path(b"", &file, 0o644));
    assert!(make_directories_path(&dir, 0o700));
    std::os::unix::fs::symlink(dir.as_str(), dir_sym.as_str()).unwrap();
    std::os::unix::fs::symlink(file.as_str(), file_sym.as_str()).unwrap();
    assert!(write_file_path(b"", &subfile, 0o644));

    let list = |p: &Path| {
        let mut entries = std::collections::BTreeMap::new();
        blaze_client::util::file::for_each_directory_entry(p, &mut |c, is_dir| {
            entries.insert(c.clone(), is_dir);
        });
        entries
    };

    // Actual test: list the directory.
    let mut expected1 = std::collections::BTreeMap::new();
    expected1.insert(dir.clone(), true);
    expected1.insert(dir_sym.clone(), false);
    expected1.insert(file.clone(), false);
    expected1.insert(file_sym.clone(), false);
    assert_eq!(expected1, list(&root));

    // Actual test: list a directory symlink.
    let mut expected2 = std::collections::BTreeMap::new();
    expected2.insert(subfile_through_sym, false);
    assert_eq!(expected2, list(&dir_sym));

    // Actual test: list a path that's actually a file, not a directory.
    assert!(list(&file).is_empty());

    // Cleanup: delete mock directory tree.
    assert!(remove_recursively(&root));
}

// Java: src/test/cpp/util/file_posix_test.cc:58 FilePosixTest.MakeDirectories
#[test]
fn make_directories_test() {
    let tmp = tmpdir();
    let tmp_dir = s(tmp.path());

    let dir = join_path(&tmp_dir, "x/y/z");
    assert!(make_directories(&dir, 0o755));

    // Changing permissions on an existing dir should work.
    assert!(make_directories(&dir, 0o750));
    let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
    assert_eq!(0o750, mode & 0o777);

    // Can't make a dir out of a file.
    let non_dir = join_path(&dir, "w");
    std::fs::write(&non_dir, "").unwrap();
    assert!(!make_directories(&non_dir, 0o755));
    assert_eq!(libc::ENOTDIR, errno());

    // Valid symlink should work.
    let symlink = join_path(&tmp_dir, "z");
    std::os::unix::fs::symlink(&dir, &symlink).unwrap();
    assert!(make_directories(&symlink, 0o755));

    // Error: Symlink to a file.
    let symlink = join_path(&tmp_dir, "w");
    std::os::unix::fs::symlink(&non_dir, &symlink).unwrap();
    assert!(!make_directories(&symlink, 0o755));
    assert_eq!(libc::ENOTDIR, errno());

    // Error: Symlink to a dir with wrong perms.
    let symlink = join_path(&tmp_dir, "s");
    std::os::unix::fs::symlink("/", &symlink).unwrap();

    // Edge cases.
    assert!(!make_directories("", 0o755));
    assert_eq!(libc::EACCES, errno());
    assert!(!make_directories("/", 0o755));
    assert_eq!(libc::EACCES, errno());
}

// Java: src/test/cpp/util/file_posix_test.cc:119 FilePosixTest.HammerMakeDirectories
#[test]
fn hammer_make_directories() {
    // The C++ test body is commented out (TODO(ulfjack)); it only builds the path.
    let tmp = tmpdir();
    let _path = join_path(&s(tmp.path()), "x/y/z");
}

// Java: src/test/cpp/util/file_posix_test.cc:129 FilePosixTest.PathExists
#[test]
fn path_exists_test() {
    assert!(!path_exists("/this/should/not/exist/mkay"));
    assert!(!path_exists("non.existent"));
    assert!(!path_exists(""));

    assert!(path_exists("/"));
    assert!(path_exists("/usr"));
    assert!(path_exists("/usr/"));
    assert!(path_exists("/usr/bin/yes"));
}

// Java: src/test/cpp/util/file_posix_test.cc:141 FilePosixTest.CanAccess
#[test]
fn can_access() {
    assert!(!can_read_file("/this/should/not/exist/mkay"));
    assert!(!can_execute_file("/this/should/not/exist/mkay"));
    assert!(!can_access_directory("/this/should/not/exist/mkay"));

    assert!(!can_read_file("non.existent"));
    assert!(!can_execute_file("non.existent"));
    assert!(!can_access_directory("non.existent"));

    let tmp = tmpdir();
    let dir = join_path(&s(tmp.path()), "canaccesstest");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();

    assert!(!can_read_file(&dir));
    assert!(!can_execute_file(&dir));
    assert!(can_access_directory(&dir));

    let file = join_path(&dir, "foo.txt");
    std::fs::write(&file, "hello").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

    assert!(can_read_file(&file));
    assert!(!can_execute_file(&file));
    assert!(!can_access_directory(&file));

    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o100)).unwrap();
    assert!(!can_read_file(&file));
    assert!(can_execute_file(&file));
    assert!(!can_access_directory(&file));

    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    assert!(!can_read_file(&dir));
    assert!(!can_execute_file(&dir));
    assert!(!can_access_directory(&dir));
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();

    std::fs::remove_file(&file).unwrap();
    std::fs::remove_dir(&dir).unwrap();
}

// Java: src/test/cpp/util/file_posix_test.cc:186 FilePosixTest.ChangeDirectory
#[test]
fn change_directory_test() {
    let _g = lock();
    // Retrieve the current working directory.
    let old_wd = s(&std::env::current_dir().unwrap());

    // Change to a different directory and assert it was successful.
    assert!(!change_directory("/non/existent/path"));
    assert!(change_directory("/usr"));
    assert_eq!("/usr", s(&std::env::current_dir().unwrap()));

    // Change back to the original CWD.
    assert!(change_directory(&old_wd));
    assert_eq!(old_wd, s(&std::env::current_dir().unwrap()));
}

// Java: src/test/cpp/util/file_posix_test.cc:263 FileTest.TestRemoveRecursivelyPosix
#[test]
fn test_remove_recursively_posix() {
    let tmp = tmpdir();
    let tempdir = Path::new(&s(tmp.path()));
    assert!(path_exists_path(&tempdir));

    let unwritable_dir = tempdir.get_relative("test_rmr_unwritable");
    assert!(make_directories_path(&unwritable_dir, 0o700));
    assert!(write_file_path(
        b"junkdata",
        &unwritable_dir.get_relative("file"),
        0o644
    ));
    std::fs::set_permissions(
        unwritable_dir.as_str(),
        std::fs::Permissions::from_mode(0o500),
    )
    .unwrap();
    assert!(!remove_recursively(&unwritable_dir));
    // Let TempDir clean up.
    std::fs::set_permissions(
        unwritable_dir.as_str(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();

    let symlink_target_dir = tempdir.get_relative("test_rmr_symlink_target_dir");
    assert!(make_directories_path(&symlink_target_dir, 0o700));
    let symlink_target_dir_file = symlink_target_dir.get_relative("file");
    assert!(write_file_path(
        b"junkdata",
        &symlink_target_dir_file,
        0o644
    ));
    let symlink_dir = tempdir.get_relative("test_rmr_symlink_dir");
    std::os::unix::fs::symlink(symlink_target_dir.as_str(), symlink_dir.as_str()).unwrap();
    assert!(remove_recursively(&symlink_dir));
    assert!(!path_exists_path(&symlink_dir));
    assert!(path_exists_path(&symlink_target_dir));
    assert!(path_exists_path(&symlink_target_dir_file));

    let dir_with_symlinks = tempdir.get_relative("test_rmr_dir_w_symlinks");
    assert!(make_directories_path(&dir_with_symlinks, 0o700));
    let file_symlink_target = tempdir.get_relative("test_rmr_dir_w_symlinks_file");
    assert!(write_file_path(b"junkdata", &file_symlink_target, 0o644));
    std::os::unix::fs::symlink(
        file_symlink_target.as_str(),
        dir_with_symlinks.get_relative("file").as_str(),
    )
    .unwrap();
    let dir_symlink_target = tempdir.get_relative("test_rmr_dir_w_symlinks_dir");
    assert!(make_directories_path(&dir_symlink_target, 0o700));
    let dir_symlink_target_file = dir_symlink_target.get_relative("file");
    assert!(write_file_path(
        b"junkdata",
        &dir_symlink_target_file,
        0o644
    ));
    std::os::unix::fs::symlink(
        dir_symlink_target.as_str(),
        dir_with_symlinks.get_relative("dir").as_str(),
    )
    .unwrap();
    assert!(remove_recursively(&dir_with_symlinks));
    assert!(!path_exists_path(&dir_with_symlinks));
    assert!(path_exists_path(&dir_symlink_target));
    assert!(path_exists_path(&dir_symlink_target_file));
    assert!(path_exists_path(&file_symlink_target));
}

// Java: src/test/cpp/util/file_posix_test.cc:309 FileTest.TestCreatSiblingTempDirDoesntClobberParentPerms
#[test]
fn test_creat_sibling_temp_dir_doesnt_clobber_parent_perms() {
    let tmp = tmpdir();
    let tempdir = Path::new(&s(tmp.path()));
    assert!(path_exists_path(&tempdir));

    let existing_parent_dir = tempdir.get_relative("existing");
    assert!(make_directories_path(&existing_parent_dir, 0o700));
    let other_dir = existing_parent_dir.get_relative("other");
    let temp_dir = create_sibling_temp_dir(&other_dir);
    assert_eq!(other_dir.get_parent(), temp_dir.get_parent());
    assert!(
        temp_dir
            .get_base_name()
            .starts_with(&(other_dir.get_base_name() + ".tmp."))
    );
    assert!(path_exists_path(&temp_dir));
    let mode = std::fs::metadata(existing_parent_dir.as_str())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(0o700, mode & 0o777);
}
