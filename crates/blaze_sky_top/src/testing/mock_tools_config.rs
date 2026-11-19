//! Port of `packages/util/MockToolsConfig.java` (src/test/java): creates the files of the mock
//! client.
//!
//! PORT: Java's `realFileSystem` mode (runfiles links and copies: `linkTool`, `copyTool`,
//! `copyDirectory`) is not ported: the mock workspace of these tests is written from the
//! literal contents in `analysis_mock.rs` only (see its module documentation).

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
