//! Port of `testutil/Scratch.java` (src/test/java): files for a test, written under a fresh
//! directory.
//!
//! PORT: Java's tests write into an `InMemoryFileSystem`; Blaze's analysis reads the real file
//! system (repository fetching, symlinks), so the scratch root is a fresh directory under the
//! system temporary directory, removed when the test ends. Java's `Scratch` resolves relative
//! names against `/workspace`; here against the workspace directory given at construction.

use std::sync::atomic::{AtomicUsize, Ordering};

use blaze_vfs::{
    DigestHashFunction, FileSystemExt, FileSystemRef, Path, PathFragment, UnixFileSystem,
};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh directory under the system temporary directory, removed on drop. Holds the
/// workspace, the output base and the other directories of one test (Java
/// `FoundationTestCase.initializeFileSystemAndDirectories`).
pub struct TestDirectory {
    pub fs: FileSystemRef,
    pub root: Path,
}

impl TestDirectory {
    pub fn new() -> TestDirectory {
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let base = std::env::temp_dir().join(format!(
            "blaze_build_view_test_{}_{}",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("create the test directory");
        // The real path, so that paths compare equal to what the file system reports.
        let base = std::fs::canonicalize(&base).expect("canonicalize the test directory");
        let root = fs.get_path(base.to_str().expect("UTF-8 temporary directory"));
        TestDirectory { fs, root }
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.root.get_path_string());
    }
}

/// `Scratch.linesAsString`: each line followed by a newline.
// Java: com/google/devtools/build/lib/testutil/Scratch.java:224 Scratch.linesAsString
pub fn lines_as_string(lines: &[&str]) -> String {
    let mut s = String::new();
    for line in lines {
        s.push_str(line);
        s.push('\n');
    }
    s
}

/// Java `Scratch`: creates files relative to a working directory.
// Java: com/google/devtools/build/lib/testutil/Scratch.java:39 Scratch
pub struct Scratch {
    fs: FileSystemRef,
    working_dir: Path,
    /// Set whenever a file changes (the harness rebuilds its evaluator before the next request;
    /// see `BuildViewTestCase`).
    modified: std::sync::atomic::AtomicBool,
}

impl Scratch {
    pub fn new(fs: FileSystemRef, working_dir: Path) -> Scratch {
        Scratch {
            fs,
            working_dir,
            modified: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Returns whether a file was written or deleted since the last call, and clears the flag.
    pub fn take_modified(&self) -> bool {
        self.modified.swap(false, Ordering::SeqCst)
    }

    /// `Scratch.resolve`: an absolute name as is, a relative one against the working directory.
    // Java: com/google/devtools/build/lib/testutil/Scratch.java:96 Scratch.resolve
    pub fn resolve(&self, path_name: &str) -> Path {
        if path_name.starts_with('/') {
            return self.fs.get_path(path_name);
        }
        self.working_dir
            .get_relative(&PathFragment::create(path_name))
    }

    /// `Scratch.dir`.
    // Java: com/google/devtools/build/lib/testutil/Scratch.java:110 Scratch.dir
    pub fn dir(&self, path_name: &str) -> Path {
        let dir = self.resolve(path_name);
        std::fs::create_dir_all(dir.get_path_string()).expect("create scratch directory");
        dir
    }

    /// `Scratch.file(pathName, lines...)`: fails if the file exists.
    // Java: com/google/devtools/build/lib/testutil/Scratch.java:121 Scratch.file
    pub fn file(&self, path_name: &str, lines: &[&str]) -> Path {
        self.file_bytes(path_name, lines_as_string(lines).as_bytes())
    }

    /// `Scratch.file(pathName, byte[] content)`.
    // Java: com/google/devtools/build/lib/testutil/Scratch.java:140 Scratch.file
    pub fn file_bytes(&self, path_name: &str, content: &[u8]) -> Path {
        let file = self.new_file(path_name);
        std::fs::write(file.get_path_string(), content).expect("write scratch file");
        self.modified.store(true, Ordering::SeqCst);
        file
    }

    /// `Scratch.overwriteFile`: like [`Scratch::file`], but the file is first deleted if it
    /// already exists.
    // Java: com/google/devtools/build/lib/testutil/Scratch.java:188 Scratch.overwriteFile
    pub fn overwrite_file(&self, path_name: &str, lines: &[&str]) -> Path {
        let old_file = self.resolve(path_name);
        let _ = std::fs::remove_file(old_file.get_path_string());
        self.file(path_name, lines)
    }

    /// `Scratch.appendFile`.
    // Java: com/google/devtools/build/lib/testutil/Scratch.java:162 Scratch.appendFile
    pub fn append_file(&self, path_name: &str, lines: &[&str]) -> Path {
        let path = self.resolve(path_name);
        let mut content = String::new();
        if path.exists() {
            content.push_str(&self.read_file(path_name));
            content.push('\n');
        }
        content.push_str(&lines_as_string(lines));
        self.overwrite_file(path_name, &[&content])
    }

    /// `Scratch.deleteFile`.
    // Java: com/google/devtools/build/lib/testutil/Scratch.java:208 Scratch.deleteFile
    pub fn delete_file(&self, path_name: &str) -> bool {
        let deleted = std::fs::remove_file(self.resolve(path_name).get_path_string()).is_ok();
        self.modified.store(true, Ordering::SeqCst);
        deleted
    }

    /// `Scratch.readFile`.
    // Java: com/google/devtools/build/lib/testutil/Scratch.java:146 Scratch.readFile
    pub fn read_file(&self, path_name: &str) -> String {
        std::fs::read_to_string(self.resolve(path_name).get_path_string())
            .expect("read scratch file")
    }

    /// Creates a new scratch file, ensuring parents exist.
    // Java: com/google/devtools/build/lib/testutil/Scratch.java:213 Scratch.newFile
    fn new_file(&self, path_name: &str) -> Path {
        let file = self.resolve(path_name);
        let parent = file
            .get_parent_directory()
            .expect("a scratch file has a parent");
        std::fs::create_dir_all(parent.get_path_string()).expect("create parent directories");
        if file.exists() {
            panic!("IOException: Could not create scratch file (file exists) {path_name}");
        }
        file
    }

    /// Sets the executable bits of a scratch file (Java `Path.chmod(0755)` on the result).
    pub fn chmod(&self, path_name: &str, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            self.resolve(path_name).get_path_string(),
            std::fs::Permissions::from_mode(mode),
        )
        .expect("chmod scratch file");
    }
}
