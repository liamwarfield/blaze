//! Port of `packages/util/MockToolsConfig.java` (src/test/java): creates the files of the mock
//! client.
//!
//! PORT: Java's `realFileSystem` mode (runfiles links and copies: `linkTool`, `copyTool`,
//! `copyDirectory`) is not ported: the mock workspace of these tests is written from the
//! literal contents in `analysis_mock.rs` only (see its module documentation).

use blaze_vfs::{Path, PathFragment};

use crate::testing::scratch::lines_as_string;

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
            let existing_content =
                std::fs::read_to_string(path.get_path_string()).expect("read mock file");
            let new_content = lines_as_string(lines);
            if new_content.trim() != existing_content.trim() {
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
            let p = std::path::Path::new(path.get_path_string());
            if p.is_dir() {
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
            .open(path.get_path_string())
            .expect("open mock file");
        f.write_all(lines_as_string(lines).as_bytes())
            .expect("append to mock file");
        path
    }

    /// `create(...).chmod(mode)`.
    pub fn chmod(&self, relative_path: &str, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            self.get_path(relative_path).get_path_string(),
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
    std::fs::create_dir_all(parent.get_path_string()).expect("create mock directories");
    std::fs::write(path.get_path_string(), lines_as_string(lines)).expect("write mock file");
}
