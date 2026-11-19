//! Port of `packages/util/MockToolsConfig.java` (src/test/java): creates the files of the mock
//! client.
//!
//! PORT: `copyTool` / `copyDirectory` read from the prepared test dependencies
//! (`test_deps.rs`) where Java reads from the runfiles. Java's tests run on an in-memory file
//! system, so `isRealFileSystem()` is false and `linkTool` is not ported.

use blaze_vfs::{Path, PathFragment};

use blaze_util::string_encoding::{latin1_decode, latin1_encode_lossy};

use crate::testing::scratch::{lines_as_string, os};

/// Java `String.trim()`: strips the chars `<= ' '`.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// Java `MockToolsConfig`.
// Java: com/google/devtools/build/lib/packages/util/MockToolsConfig.java:31 MockToolsConfig
pub struct MockToolsConfig {
    root_directory: Path,
}

impl MockToolsConfig {
    pub fn new(root_directory: Path) -> MockToolsConfig {
        MockToolsConfig { root_directory }
    }

    // Java: com/google/devtools/build/lib/packages/util/MockToolsConfig.java:64 MockToolsConfig.getPath
    pub fn get_path(&self, relative_path: &str) -> Path {
        blaze_util::check_state!(!relative_path.starts_with('/'), "{}", relative_path);
        self.root_directory
            .get_relative(&PathFragment::create(relative_path))
    }

    /// Creates the file with the given lines; an existing file must have the same content.
    // Java: com/google/devtools/build/lib/packages/util/MockToolsConfig.java:69 MockToolsConfig.create
    pub fn create(&self, relative_path: &str, lines: &[&str]) -> Path {
        let path = self.get_path(relative_path);
        if !path.exists() {
            write_lines(&path, lines);
        } else if !lines.is_empty() {
            // new String(FileSystemUtils.readContentAsLatin1(path))
            let existing_content =
                latin1_decode(&std::fs::read(os(&path)).expect("read mock file")).into_owned();
            let new_content = lines_as_string(lines);
            if java_trim(&new_content) != java_trim(&existing_content) {
                panic!(
                    "IOException: Conflict: '{relative_path}':\n'{new_content}'\n vs \n'{existing_content}'"
                );
            }
        }
        path
    }

    // Java: com/google/devtools/build/lib/packages/util/MockToolsConfig.java:94 MockToolsConfig.overwrite
    pub fn overwrite(&self, relative_path: &str, lines: &[&str]) -> Path {
        let path = self.get_path(relative_path);
        if path.exists() {
            let p = os(&path);
            if std::path::Path::new(&p).is_dir() {
                std::fs::remove_dir_all(p).expect("delete mock tree");
            } else {
                std::fs::remove_file(p).expect("delete mock file");
            }
        }
        self.create(relative_path, lines)
    }

    // Java: com/google/devtools/build/lib/packages/util/MockToolsConfig.java:102 MockToolsConfig.append
    pub fn append(&self, relative_path: &str, lines: &[&str]) -> Path {
        if relative_path == "WORKSPACE" {
            panic!(
                "IllegalArgumentException: DO NOT write into the WORKSPACE file in mocks. Use \
                 MODULE.bazel instead"
            );
        }
        let path = self.get_path(relative_path);
        if !path.exists() {
            return self.create(relative_path, lines);
        }
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(os(&path))
            .expect("open mock file");
        // FileSystemUtils.appendIsoLatin1
        f.write_all(&latin1_encode_lossy(&lines_as_string(lines)))
            .expect("append to mock file");
        path
    }

    /// `isRealFileSystem()`: Java's analysis tests run on an in-memory file system.
    // Java: com/google/devtools/build/lib/packages/util/MockToolsConfig.java:62 MockToolsConfig.isRealFileSystem
    pub fn is_real_file_system(&self) -> bool {
        false
    }

    /// `copyTool(rlocationPath, dest)`: overwrites `dest` with the lines of `source` (read as
    /// Latin-1).
    // Java: com/google/devtools/build/lib/packages/util/MockToolsConfig.java:171 MockToolsConfig.copyTool
    pub fn copy_tool(&self, source: &std::path::Path, dest: &str) {
        let bytes = std::fs::read(source)
            .unwrap_or_else(|e| panic!("IOException: {}: {e}", source.display()));
        let content = latin1_decode(&bytes);
        let lines = guava_read_lines(&content);
        let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
        self.overwrite(dest, &lines);
    }

    /// `copyDirectory(path, to, depth, useEmptyBuildFiles)`: copies every file under `source`
    /// (up to `depth` levels) to `to`; a file whose name ends in `BUILD` is created empty when
    /// `use_empty_build_files`.
    // Java: com/google/devtools/build/lib/packages/util/MockToolsConfig.java:214 MockToolsConfig.copyDirectory
    pub fn copy_directory(
        &self,
        source: &std::path::Path,
        to: &str,
        depth: usize,
        use_empty_build_files: bool,
    ) {
        // Files.walk(source, depth).filter(isFile): directories are followed to `depth`;
        // Files.walk does not follow symbolic links, and File.isFile follows them.
        fn walk(dir: &std::path::Path, depth: usize, out: &mut Vec<std::path::PathBuf>) {
            if depth == 0 {
                return;
            }
            let mut entries: Vec<_> = std::fs::read_dir(dir)
                .unwrap_or_else(|e| panic!("IOException: {}: {e}", dir.display()))
                .map(|e| e.expect("directory entry").path())
                .collect();
            entries.sort();
            for entry in entries {
                let meta = std::fs::symlink_metadata(&entry).expect("metadata");
                if meta.is_dir() {
                    walk(&entry, depth - 1, out);
                } else if entry.is_file() {
                    out.push(entry);
                }
            }
        }
        let mut files = Vec::new();
        walk(source, depth, &mut files);
        for file in files {
            let relative = file
                .strip_prefix(source)
                .expect("under the source")
                .to_str()
                .expect("an ASCII path of the prepared test dependencies")
                .to_string();
            if relative.is_empty() {
                continue;
            }
            let dest = format!("{to}/{relative}");
            if relative.ends_with("BUILD") && use_empty_build_files {
                self.create(&dest, &[]);
            } else {
                self.copy_tool(&file, &dest);
            }
        }
    }

    /// `create(...).chmod(mode)`.
    pub fn chmod(&self, relative_path: &str, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            os(&self.get_path(relative_path)),
            std::fs::Permissions::from_mode(mode),
        )
        .expect("chmod mock file");
    }
}

/// `FileSystemUtils.writeIsoLatin1`: every line followed by `\n`.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:723 FileSystemUtils.writeLinesAs
fn write_lines(path: &Path, lines: &[&str]) {
    let parent = path
        .get_parent_directory()
        .expect("a mock file has a parent");
    std::fs::create_dir_all(os(&parent)).expect("create mock directories");
    std::fs::write(os(path), latin1_encode_lossy(&lines_as_string(lines)))
        .expect("write mock file");
}

/// Guava `CharSource.readLines()`: the lines without their terminators (`\r\n`, `\n` or `\r`);
/// a final terminator does not start another line.
fn guava_read_lines(content: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut chars = content.chars().peekable();
    let mut pending = false;
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                lines.push(std::mem::take(&mut current));
                pending = false;
            }
            '\n' => {
                lines.push(std::mem::take(&mut current));
                pending = false;
            }
            _ => {
                current.push(c);
                pending = true;
            }
        }
    }
    if pending {
        lines.push(current);
    }
    lines
}
