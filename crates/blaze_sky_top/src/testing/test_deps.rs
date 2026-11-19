//! The repositories and resource files that Bazel's analysis mocks read from the test's runfiles
//! (`Runfiles.rlocation("rules_cc/...")`, `ResourceLoader.readFromResources(...)`).
//!
//! PORT: a hermetic Rust test has no runfiles; `scripts/test-deps/prepare` materialises the same
//! repositories (the versions the pinned Bazel's own build selects, fetched by the oracle) and
//! the resource files of the pinned source into one directory with a `manifest.json`. The tests
//! find it through the environment variable `BLAZE_TEST_DEPS` (default
//! `~/.cache/blaze/test-deps`). A test that needs it and does not find it fails, naming the
//! script: it never skips.

use std::path::PathBuf;
use std::sync::OnceLock;

/// The script that prepares the directory.
pub const PREPARE_SCRIPT: &str = "scripts/test-deps/prepare";

/// The prepared directory and its manifest.
pub struct TestDeps {
    root: PathBuf,
    manifest: serde_json::Value,
}

fn missing(what: &str) -> ! {
    panic!(
        "the analysis test mocks need the prepared test dependencies ({what}); run \
         `{PREPARE_SCRIPT}` (it writes ~/.cache/blaze/test-deps), or set BLAZE_TEST_DEPS to the \
         directory it wrote"
    )
}

/// The prepared directory (`BLAZE_TEST_DEPS`, default `~/.cache/blaze/test-deps`); fails when it
/// or its manifest is missing.
pub fn test_deps() -> &'static TestDeps {
    static DEPS: OnceLock<TestDeps> = OnceLock::new();
    DEPS.get_or_init(|| {
        let root = match std::env::var_os("BLAZE_TEST_DEPS") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => match std::env::var_os("HOME") {
                Some(home) => PathBuf::from(home).join(".cache/blaze/test-deps"),
                None => missing("neither BLAZE_TEST_DEPS nor HOME is set"),
            },
        };
        let manifest_path = root.join("manifest.json");
        let text = match std::fs::read(&manifest_path) {
            Ok(t) => t,
            Err(e) => missing(&format!("cannot read {}: {e}", manifest_path.display())),
        };
        let manifest: serde_json::Value = match serde_json::from_slice(&text) {
            Ok(m) => m,
            Err(e) => missing(&format!("{} is not JSON: {e}", manifest_path.display())),
        };
        TestDeps { root, manifest }
    })
}

impl TestDeps {
    /// The root of the prepared repository `name` (a Bazel module name, e.g. `rules_cc`): the
    /// directory Java's `runfiles.rlocation("<repo name>/...")` resolves into.
    pub fn repository(&self, name: &str) -> PathBuf {
        let directory = self.manifest["repositories"][name]["directory"]
            .as_str()
            .unwrap_or_else(|| missing(&format!("repository {name} is not in the manifest")));
        let dir = self.root.join(directory);
        if !dir.is_dir() {
            missing(&format!("{} does not exist", dir.display()));
        }
        dir
    }

    /// A file of the pinned Bazel source that the mocks read as a test resource (a path relative
    /// to the source root, e.g. `tools/python/utils.bzl`).
    pub fn resource(&self, path: &str) -> PathBuf {
        if self.manifest["resources"][path].as_str().is_none() {
            missing(&format!("resource {path} is not in the manifest"));
        }
        let file = self.root.join("bazel-resources").join(path);
        if !file.is_file() {
            missing(&format!("{} does not exist", file.display()));
        }
        file
    }

    /// `ResourceLoader.readFromResources(path)`: the file read as UTF-8.
    // Java: com/google/devtools/build/lib/packages/util/ResourceLoader.java:29 ResourceLoader.readFromResources
    pub fn read_resource(&self, file: &std::path::Path) -> String {
        let bytes = std::fs::read(file)
            .unwrap_or_else(|e| missing(&format!("cannot read {}: {e}", file.display())));
        // new String(bytes, UTF_8): malformed input becomes U+FFFD.
        String::from_utf8_lossy(&bytes).into_owned()
    }
}
