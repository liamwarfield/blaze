//! Port of `bazel/bzlmod/FakeRegistry.java` (src/test/java): the registry of `BuildViewTestCase`
//! (`registry`), to which a test adds modules with `addModule`.
//!
//! PORT: Java plugs a fake `Registry` into `ModuleFileFunction` (`FakeRegistry.Factory`, URL
//! `fake:0`). Blaze's executor creates its registries from `--registry` like the binary, so the
//! fake registry is laid out on disk as an index registry at `moduleRoot` (the harness's
//! `--registry=file://<moduleRoot>`): `addModule` writes `modules/<name>/<version>/MODULE.bazel`
//! with the given lines and a `source.json` of type `local_path` pointing at
//! `<moduleRoot>/<name>+<version>`, which `IndexRegistry` turns into the same
//! `LocalPathRepoSpecs.create(rootPath + "/" + name + "+" + version)` as `FakeRegistry.getRepoSpec`.
//! Differences: the registry URL in events, messages and the lockfile is `file://<moduleRoot>`
//! instead of `fake:0`, and `IndexRegistry` also asks for `bazel_registry.json` and
//! `modules/<name>/metadata.json` (absent: no yanked versions, as `FakeRegistry` without
//! `addYankedVersion`).

use blaze_bzlmod_model::{ModuleKey, Version};
use blaze_vfs::Path;

use crate::testing::scratch::Scratch;

/// Java `FakeRegistry` (see the module documentation).
// Java: com/google/devtools/build/lib/bazel/bzlmod/FakeRegistry.java:43 FakeRegistry
pub struct FakeRegistry<'a> {
    scratch: &'a Scratch,
    /// `rootPath` (`moduleRoot`).
    root_path: &'a Path,
}

impl<'a> FakeRegistry<'a> {
    pub(crate) fn new(scratch: &'a Scratch, root_path: &'a Path) -> FakeRegistry<'a> {
        FakeRegistry { scratch, root_path }
    }

    /// `getUrl()`.
    pub fn get_url(&self) -> String {
        format!("file://{}", self.root_path.get_path_string())
    }

    /// `addModule(key, moduleFileLines...)`: the module file is the lines joined with `\n`
    /// (UTF-8, as `FakeRegistry.getModuleFile`).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/FakeRegistry.java:56 FakeRegistry.addModule
    pub fn add_module(&self, key: &ModuleKey, module_file_lines: &[&str]) -> &Self {
        let dir = format!(
            "{}/modules/{}/{}",
            self.root_path.get_path_string(),
            key.name(),
            key.version()
        );
        self.scratch.overwrite_file_bytes(
            &format!("{dir}/MODULE.bazel"),
            module_file_lines.join("\n").as_bytes(),
        );
        // Java: FakeRegistry.getRepoSpec.
        let path = format!(
            "{}/{}",
            self.root_path.get_path_string(),
            key.get_canonical_repo_name_with_version().get_name()
        );
        let source_json = format!(
            "{{\"type\": \"local_path\", \"path\": {}}}",
            serde_json::Value::String(path)
        );
        self.scratch
            .overwrite_file_bytes(&format!("{dir}/source.json"), source_json.as_bytes());
        self
    }
}

/// `BzlmodTestUtil.createModuleKey(name, version)`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BzlmodTestUtil.java:41 BzlmodTestUtil.createModuleKey
pub fn create_module_key(name: &str, version: &str) -> ModuleKey {
    ModuleKey::new(
        name,
        Version::parse(version).unwrap_or_else(|e| panic!("ParseException: {e:?}")),
    )
}
