//! Ports of `IndexRegistryTest` and `RegistryFactoryTest`. The Java tests serve files from a
//! local HTTP server through `DownloadManager`; here a fake `RegistryFetcher` serves the same
//! files and models the download cache (checksum-addressed) that the checksum tests rely on.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use blaze_bzlmod_model::registry::verify_checksum;
use blaze_bzlmod_model::{
    ArchiveRepoSpecBuilder, Checksum, FileRegistryFetcher, GitRepoSpecBuilder, JavaUri,
    LockfileMode, ModuleFile, Registry, RegistryException, RegistryFactory, RegistryFactoryImpl,
    RegistryFetcher, RegistryFileDownloadEvent, RemoteFile, Version, local_path_repo_spec,
};
use blaze_events::StoredEventHandler;
use common::*;
use indexmap::{IndexMap, IndexSet};

const SERVER: &str = "http://localhost:8080";

/// `TestHttpServer` + `DownloadManager` with a `DownloadCache`.
#[derive(Default)]
struct FakeServer {
    files: Mutex<HashMap<String, Vec<u8>>>,
    cache_enabled: bool,
    cache: Mutex<HashMap<String, Vec<u8>>>,
}

impl FakeServer {
    fn with_cache() -> Self {
        FakeServer {
            cache_enabled: true,
            ..Default::default()
        }
    }

    fn serve(&self, path: &str, lines: &[&str]) {
        self.serve_bytes(path, lines.join("\n").into_bytes());
    }

    fn serve_bytes(&self, path: &str, content: Vec<u8>) {
        self.files
            .lock()
            .unwrap()
            .insert(format!("{SERVER}{path}"), content);
    }

    fn unserve(&self, path: &str) {
        self.files
            .lock()
            .unwrap()
            .remove(&format!("{SERVER}{path}"));
    }
}

impl RegistryFetcher for FakeServer {
    fn download_and_read_one_url_for_bzlmod(
        &self,
        url: &JavaUri,
        _client_env: &IndexMap<String, String>,
        checksum: Option<&Checksum>,
    ) -> Result<Vec<u8>, RegistryException> {
        if self.cache_enabled
            && let Some(c) = checksum
            && let Some(content) = self.cache.lock().unwrap().get(&c.to_string())
        {
            return Ok(content.clone());
        }
        let Some(content) = self.files.lock().unwrap().get(&url.to_string()).cloned() else {
            return Err(RegistryException::NotFound(url.to_string()));
        };
        verify_checksum(&content, checksum)?;
        if self.cache_enabled {
            let key = match checksum {
                Some(c) => c.to_string(),
                None => Checksum::sha256_of(&content).to_string(),
            };
            self.cache.lock().unwrap().insert(key, content.clone());
        }
        Ok(content)
    }
}

fn factory() -> RegistryFactoryImpl {
    RegistryFactoryImpl::new(Box::new(IndexMap::new))
}

fn create(
    url: &str,
    mode: LockfileMode,
    known: &IndexMap<String, Option<Checksum>>,
    mirrors: &[&str],
) -> Arc<dyn Registry> {
    factory()
        .create_registry(
            url,
            mode,
            known,
            &IndexMap::new(),
            None,
            &mirrors
                .iter()
                .map(|s| s.to_string())
                .collect::<IndexSet<_>>(),
        )
        .unwrap()
}

fn sha256(content: &str) -> Checksum {
    Checksum::sha256_of(content.as_bytes())
}

fn recorded(handler: &StoredEventHandler) -> Vec<(String, Option<String>)> {
    RegistryFileDownloadEvent::collect_to_map(&handler.get_posts())
        .into_iter()
        .map(|(k, v)| (k, v.map(|c| c.to_string())))
        .collect()
}

fn recorded_checksums(handler: &StoredEventHandler) -> IndexMap<String, Option<Checksum>> {
    RegistryFileDownloadEvent::collect_to_map(&handler.get_posts())
}

#[test]
fn test_http_url() {
    let server = FakeServer::default();
    server.serve("/myreg/modules/foo/1.0/MODULE.bazel", &["lol"]);
    let reporter = StoredEventHandler::new();
    let registry = create(
        &format!("{SERVER}/myreg"),
        LockfileMode::Update,
        &IndexMap::new(),
        &[],
    );
    assert_eq!(
        registry
            .get_module_file(&create_module_key("foo", "1.0"), &reporter, &server)
            .unwrap(),
        ModuleFile::create(
            "lol".as_bytes(),
            format!("{SERVER}/myreg/modules/foo/1.0/MODULE.bazel")
        )
    );
    assert!(matches!(
        registry.get_module_file(&create_module_key("bar", "1.0"), &reporter, &server),
        Err(RegistryException::NotFound(_))
    ));
}

#[test]
#[ignore = "needs the HTTP downloader and netrc credentials (DownloadManager)"]
fn test_http_url_with_netrc_creds() {}

#[test]
fn test_file_url() {
    let root = std::env::temp_dir().join(format!(
        "blaze_bzlmod_model_index_registry_{}",
        std::process::id()
    ));
    let dir = root.join("fakereg/modules/foo/1.0");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("MODULE.bazel");
    std::fs::write(&file, "lol").unwrap();
    // new File(...).toURI(): "file:" + absolute path, with a trailing '/' for directories.
    let reg_url = format!("file:{}/", root.join("fakereg").display());
    let reporter = StoredEventHandler::new();
    let registry = create(&reg_url, LockfileMode::Update, &IndexMap::new(), &[]);
    assert_eq!(
        registry
            .get_module_file(
                &create_module_key("foo", "1.0"),
                &reporter,
                &FileRegistryFetcher
            )
            .unwrap(),
        ModuleFile::create("lol".as_bytes(), format!("file:{}", file.display()))
    );
    assert!(matches!(
        registry.get_module_file(
            &create_module_key("bar", "1.0"),
            &reporter,
            &FileRegistryFetcher
        ),
        Err(RegistryException::NotFound(_))
    ));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn test_get_archive_repo_spec() {
    let server = FakeServer::default();
    server.serve(
        "/bazel_registry.json",
        &[
            "{",
            "  \"mirrors\": [",
            "    \"https://mirror.bazel.build/\",",
            "    \"file:///home/bazel/mymirror/\"",
            "  ]",
            "}",
        ],
    );
    server.serve(
        "/modules/foo/1.0/source.json",
        &[
            "{",
            "  \"url\": \"http://mysite.com/thing.zip\",",
            "  \"mirror_urls\": [\"http://my.mirror/mysite.com/thing.zip\",\"http://another.mirror/mysite.com/thing.zip\"],",
            "  \"integrity\": \"sha256-blah\",",
            "  \"strip_prefix\": \"pref\"",
            "}",
        ],
    );
    server.serve(
        "/modules/foo/1.0/MODULE.bazel",
        &["module(name = \"foo\", version = \"1.0\")"],
    );
    server.serve(
        "/modules/bar/2.0/source.json",
        &[
            "{",
            "  \"url\": \"https://example.com/archive.jar?with=query\",",
            "  \"integrity\": \"sha256-bleh\",",
            "  \"patches\": {",
            "    \"1.fix-this.patch\": \"sha256-lol\",",
            "    \"2.fix-that.patch\": \"sha256-kek\"",
            "  },",
            "  \"patch_strip\": 3",
            "}",
        ],
    );
    server.serve(
        "/modules/bar/2.0/MODULE.bazel",
        &["module(name = \"bar\", version = \"2.0\")"],
    );
    server.serve(
        "/modules/baz/3.0/source.json",
        &[r#"{
    "url": "https://example.com/archive.jar?with=query",
    "integrity": "sha256-bleh",
    "overlay": {
        "BUILD.bazel": "sha256-bleh-overlay"
    }
}
"#],
    );
    server.serve(
        "/modules/baz/3.0/MODULE.bazel",
        &["module(name = \"baz\", version = \"3.0\")"],
    );
    let module_file_registry_hashes: IndexMap<String, Option<Checksum>> = IndexMap::from([
        (
            format!("{SERVER}/modules/foo/1.0/MODULE.bazel"),
            Some(sha256("module(name = \"foo\", version = \"1.0\")")),
        ),
        (
            format!("{SERVER}/modules/baz/3.0/MODULE.bazel"),
            Some(sha256("module(name = \"baz\", version = \"3.0\")")),
        ),
        (
            format!("{SERVER}/modules/bar/2.0/MODULE.bazel"),
            Some(sha256("module(name = \"bar\", version = \"2.0\")")),
        ),
    ]);
    let reporter = StoredEventHandler::new();
    let registry = create(
        SERVER,
        LockfileMode::Update,
        &IndexMap::new(),
        &["https://my.mirror"],
    );
    assert_eq!(
        registry
            .get_repo_spec(
                &create_module_key("foo", "1.0"),
                &module_file_registry_hashes,
                &reporter,
                &server
            )
            .unwrap(),
        ArchiveRepoSpecBuilder::new()
            .set_urls(vec![
                "https://my.mirror/mysite.com/thing.zip".into(),
                "https://mirror.bazel.build/mysite.com/thing.zip".into(),
                "file:///home/bazel/mymirror/mysite.com/thing.zip".into(),
                "http://mysite.com/thing.zip".into(),
                "http://my.mirror/mysite.com/thing.zip".into(),
                "http://another.mirror/mysite.com/thing.zip".into(),
            ])
            .set_integrity("sha256-blah")
            .set_strip_prefix("pref")
            .set_remote_patches(IndexMap::new())
            .set_overlay(IndexMap::new())
            .set_remote_module_file(RemoteFile::new(
                sha256("module(name = \"foo\", version = \"1.0\")").to_subresource_integrity(),
                vec![format!("{SERVER}/modules/foo/1.0/MODULE.bazel")]
            ))
            .set_remote_patch_strip(0)
            .build()
    );
    assert_eq!(
        registry
            .get_repo_spec(
                &create_module_key("bar", "2.0"),
                &module_file_registry_hashes,
                &reporter,
                &server
            )
            .unwrap(),
        ArchiveRepoSpecBuilder::new()
            .set_urls(vec![
                "https://my.mirror/example.com/archive.jar?with=query".into(),
                "https://mirror.bazel.build/example.com/archive.jar?with=query".into(),
                "file:///home/bazel/mymirror/example.com/archive.jar?with=query".into(),
                "https://example.com/archive.jar?with=query".into(),
            ])
            .set_integrity("sha256-bleh")
            .set_strip_prefix("")
            .set_remote_patches(IndexMap::from([
                (
                    format!("{SERVER}/modules/bar/2.0/patches/1.fix-this.patch"),
                    "sha256-lol".to_string()
                ),
                (
                    format!("{SERVER}/modules/bar/2.0/patches/2.fix-that.patch"),
                    "sha256-kek".to_string()
                ),
            ]))
            .set_remote_patch_strip(3)
            .set_overlay(IndexMap::new())
            .set_remote_module_file(RemoteFile::new(
                sha256("module(name = \"bar\", version = \"2.0\")").to_subresource_integrity(),
                vec![format!("{SERVER}/modules/bar/2.0/MODULE.bazel")]
            ))
            .build()
    );
    assert_eq!(
        registry
            .get_repo_spec(
                &create_module_key("baz", "3.0"),
                &module_file_registry_hashes,
                &reporter,
                &server
            )
            .unwrap(),
        ArchiveRepoSpecBuilder::new()
            .set_urls(vec![
                "https://my.mirror/example.com/archive.jar?with=query".into(),
                "https://mirror.bazel.build/example.com/archive.jar?with=query".into(),
                "file:///home/bazel/mymirror/example.com/archive.jar?with=query".into(),
                "https://example.com/archive.jar?with=query".into(),
            ])
            .set_integrity("sha256-bleh")
            .set_strip_prefix("")
            .set_overlay(IndexMap::from([(
                "BUILD.bazel".to_string(),
                RemoteFile::new(
                    "sha256-bleh-overlay",
                    vec![format!("{SERVER}/modules/baz/3.0/overlay/BUILD.bazel")]
                )
            )]))
            .set_remote_module_file(RemoteFile::new(
                sha256("module(name = \"baz\", version = \"3.0\")").to_subresource_integrity(),
                vec![format!("{SERVER}/modules/baz/3.0/MODULE.bazel")]
            ))
            .set_remote_patches(IndexMap::new())
            .set_remote_patch_strip(0)
            .build()
    );
}

#[test]
fn test_get_git_repo_spec() {
    let server = FakeServer::default();
    server.serve(
        "/bazel_registry.json",
        &[r#"{
  "mirrors": [
    "https://mirror.bazel.build/",
    "file:///home/bazel/mymirror/"
  ]
}
"#],
    );
    server.serve(
        "/modules/foo/1.0/source.json",
        &[r#"{
    "type": "git_repository",
    "remote": "https://github.com/raspberrypi/pico-sdk.git",
    "commit": "4b6e647590213f253f2789ad9026df1d00f38c5d",
    "patches": {
        "foo.patch": "sha256-totallyarealhash"
    },
    "patch_strip": 1
}
"#],
    );
    server.serve(
        "/modules/foo/1.0/MODULE.bazel",
        &["module(name = \"foo\", version = \"1.0\")"],
    );
    let hashes = IndexMap::from([(
        format!("{SERVER}/modules/foo/1.0/MODULE.bazel"),
        Some(sha256("module(name = \"foo\", version = \"1.0\")")),
    )]);
    let registry = create(SERVER, LockfileMode::Update, &IndexMap::new(), &[]);
    assert_eq!(
        registry
            .get_repo_spec(
                &create_module_key("foo", "1.0"),
                &hashes,
                &StoredEventHandler::new(),
                &server
            )
            .unwrap(),
        GitRepoSpecBuilder::new()
            .set_remote(Some("https://github.com/raspberrypi/pico-sdk.git"))
            .set_commit(Some("4b6e647590213f253f2789ad9026df1d00f38c5d"))
            .set_init_submodules(false)
            .set_verbose(false)
            .set_remote_module_file(&RemoteFile::new(
                sha256("module(name = \"foo\", version = \"1.0\")").to_subresource_integrity(),
                vec![format!("{SERVER}/modules/foo/1.0/MODULE.bazel")]
            ))
            .set_remote_patches(&IndexMap::from([(
                format!("{SERVER}/modules/foo/1.0/patches/foo.patch"),
                "sha256-totallyarealhash".to_string()
            )]))
            .set_remote_patch_strip(1)
            .build()
    );
}

#[test]
fn test_get_local_path_repo_spec() {
    let server = FakeServer::default();
    server.serve(
        "/bazel_registry.json",
        &["{", "  \"module_base_path\": \"/hello/foo\"", "}"],
    );
    server.serve(
        "/modules/foo/1.0/source.json",
        &[
            "{",
            "  \"type\": \"local_path\",",
            "  \"path\": \"../bar/project_x\"",
            "}",
        ],
    );
    let registry = create(SERVER, LockfileMode::Update, &IndexMap::new(), &[]);
    assert_eq!(
        registry
            .get_repo_spec(
                &create_module_key("foo", "1.0"),
                &IndexMap::new(),
                &StoredEventHandler::new(),
                &server
            )
            .unwrap(),
        local_path_repo_spec("/hello/bar/project_x")
    );
}

#[test]
fn test_get_repo_invalid_registry_json_spec() {
    let server = FakeServer::default();
    server.serve("/bazel_registry.json", &["", "", "", ""]);
    server.serve(
        "/modules/foo/1.0/source.json",
        &[
            "{",
            "  \"url\": \"http://mysite.com/thing.zip\",",
            "  \"integrity\": \"sha256-blah\",",
            "  \"strip_prefix\": \"pref\"",
            "}",
        ],
    );
    server.serve(
        "/modules/foo/1.0/MODULE.bazel",
        &["module(name = \"foo\", version = \"1.0\")"],
    );
    let registry = create(SERVER, LockfileMode::Update, &IndexMap::new(), &[]);
    assert_eq!(
        registry
            .get_repo_spec(
                &create_module_key("foo", "1.0"),
                &IndexMap::from([(
                    format!("{SERVER}/modules/foo/1.0/MODULE.bazel"),
                    Some(sha256("module(name = \"foo\", version = \"1.0\")"))
                )]),
                &StoredEventHandler::new(),
                &server
            )
            .unwrap(),
        ArchiveRepoSpecBuilder::new()
            .set_urls(vec!["http://mysite.com/thing.zip".into()])
            .set_integrity("sha256-blah")
            .set_strip_prefix("pref")
            .set_remote_patches(IndexMap::new())
            .set_overlay(IndexMap::new())
            .set_remote_module_file(RemoteFile::new(
                sha256("module(name = \"foo\", version = \"1.0\")").to_subresource_integrity(),
                vec![format!("{SERVER}/modules/foo/1.0/MODULE.bazel")]
            ))
            .set_remote_patch_strip(0)
            .build()
    );
}

#[test]
fn test_get_repo_invalid_module_json_spec() {
    let server = FakeServer::default();
    server.serve(
        "/bazel_registry.json",
        &[
            "{",
            "  \"mirrors\": [",
            "    \"https://mirror.bazel.build/\",",
            "    \"file:///home/bazel/mymirror/\"",
            "  ]",
            "}",
        ],
    );
    server.serve(
        "/modules/foo/1.0/source.json",
        &[
            "{",
            "  \"url\": \"http://mysite.com/thing.zip\",",
            "  \"integrity\": \"sha256-blah\",",
            "  \"strip_prefix\": \"pref\",",
            "}",
        ],
    );
    let registry = create(SERVER, LockfileMode::Update, &IndexMap::new(), &[]);
    assert!(matches!(
        registry.get_repo_spec(
            &create_module_key("foo", "1.0"),
            &IndexMap::new(),
            &StoredEventHandler::new(),
            &server
        ),
        Err(RegistryException::Io(_))
    ));
}

#[test]
fn test_get_yanked_version() {
    let server = FakeServer::default();
    server.serve(
        "/modules/red-pill/metadata.json",
        &["{\n    'homepage': 'https://docs.matrix.org/red-pill',\n    'maintainers': [\n        {\n            'email': 'neo@matrix.org',\n            'github': 'neo',\n            'name': 'Neo'\n        }\n    ],\n    'versions': [\n        '1.0',\n        '2.0'\n    ],\n    'yanked_versions': {        '1.0': 'red-pill 1.0 is yanked due to CVE-2000-101, please upgrade to 2.0'\n    }\n}"],
    );
    let registry = create(SERVER, LockfileMode::Update, &IndexMap::new(), &[]);
    let yanked = registry
        .get_yanked_versions("red-pill", &StoredEventHandler::new(), &server)
        .unwrap();
    assert_eq!(
        yanked,
        Some(IndexMap::from([(
            Version::parse("1.0").unwrap(),
            "red-pill 1.0 is yanked due to CVE-2000-101, please upgrade to 2.0".to_string()
        )]))
    );
}

#[test]
fn test_archive_with_explicit_type() {
    let server = FakeServer::default();
    server.serve(
        "/modules/archive_type/1.0/source.json",
        &[
            "{",
            "  \"url\": \"https://mysite.com/thing?format=zip\",",
            "  \"integrity\": \"sha256-blah\",",
            "  \"archive_type\": \"zip\"",
            "}",
        ],
    );
    server.serve(
        "/modules/archive_type/1.0/MODULE.bazel",
        &["module(name = \"archive_type\", version = \"1.0\")"],
    );
    let registry = create(SERVER, LockfileMode::Update, &IndexMap::new(), &[]);
    assert_eq!(
        registry
            .get_repo_spec(
                &create_module_key("archive_type", "1.0"),
                &IndexMap::from([(
                    format!("{SERVER}/modules/archive_type/1.0/MODULE.bazel"),
                    Some(sha256("module(name = \"archive_type\", version = \"1.0\")"))
                )]),
                &StoredEventHandler::new(),
                &server
            )
            .unwrap(),
        ArchiveRepoSpecBuilder::new()
            .set_urls(vec!["https://mysite.com/thing?format=zip".into()])
            .set_integrity("sha256-blah")
            .set_strip_prefix("")
            .set_archive_type(Some("zip"))
            .set_remote_patches(IndexMap::new())
            .set_remote_patch_strip(0)
            .set_overlay(IndexMap::new())
            .set_remote_module_file(RemoteFile::new(
                sha256("module(name = \"archive_type\", version = \"1.0\")")
                    .to_subresource_integrity(),
                vec![format!("{SERVER}/modules/archive_type/1.0/MODULE.bazel")]
            ))
            .build()
    );
}

#[test]
fn test_get_module_file_checksums() {
    let server = FakeServer::with_cache();
    server.serve("/myreg/modules/foo/1.0/MODULE.bazel", &["old"]);
    server.serve("/myreg/modules/foo/2.0/MODULE.bazel", &["new"]);
    let known_files = IndexMap::from([
        (
            format!("{SERVER}/myreg/modules/foo/1.0/MODULE.bazel"),
            Some(sha256("old")),
        ),
        (
            format!("{SERVER}/myreg/modules/unused/1.0/MODULE.bazel"),
            Some(sha256("unused")),
        ),
    ]);
    let reporter = StoredEventHandler::new();
    let registry = create(
        &format!("{SERVER}/myreg"),
        LockfileMode::Update,
        &known_files,
        &[],
    );
    assert_eq!(
        registry
            .get_module_file(&create_module_key("foo", "1.0"), &reporter, &server)
            .unwrap(),
        ModuleFile::create(
            "old".as_bytes(),
            format!("{SERVER}/myreg/modules/foo/1.0/MODULE.bazel")
        )
    );
    assert_eq!(
        registry
            .get_module_file(&create_module_key("foo", "2.0"), &reporter, &server)
            .unwrap(),
        ModuleFile::create(
            "new".as_bytes(),
            format!("{SERVER}/myreg/modules/foo/2.0/MODULE.bazel")
        )
    );
    let e = registry
        .get_module_file(&create_module_key("bar", "1.0"), &reporter, &server)
        .unwrap_err();
    assert_eq!(
        e,
        RegistryException::NotFound(format!(
            "{SERVER}/myreg/modules/bar/1.0/MODULE.bazel: not found"
        ))
    );
    assert_eq!(
        recorded(&reporter),
        vec![
            (
                format!("{SERVER}/myreg/modules/foo/1.0/MODULE.bazel"),
                Some(sha256("old").to_string())
            ),
            (
                format!("{SERVER}/myreg/modules/foo/2.0/MODULE.bazel"),
                Some(sha256("new").to_string())
            ),
            (format!("{SERVER}/myreg/modules/bar/1.0/MODULE.bazel"), None),
        ]
    );

    let registry2 = create(
        &format!("{SERVER}/myreg"),
        LockfileMode::Update,
        &recorded_checksums(&reporter),
        &[],
    );
    // The recorded hashes are used for repo cache hits even when the server content changes.
    server.unserve("/myreg/modules/foo/1.0/MODULE.bazel");
    server.unserve("/myreg/modules/foo/2.0/MODULE.bazel");
    server.serve("/myreg/modules/bar/1.0/MODULE.bazel", &["no longer 404"]);
    let reporter2 = StoredEventHandler::new();
    assert_eq!(
        registry2
            .get_module_file(&create_module_key("foo", "1.0"), &reporter2, &server)
            .unwrap()
            .content,
        b"old"
    );
    assert_eq!(
        registry2
            .get_module_file(&create_module_key("foo", "2.0"), &reporter2, &server)
            .unwrap()
            .content,
        b"new"
    );
    let e = registry2
        .get_module_file(&create_module_key("bar", "1.0"), &reporter2, &server)
        .unwrap_err();
    assert_eq!(
        e.get_message(),
        format!(
            "{SERVER}/myreg/modules/bar/1.0/MODULE.bazel: previously not found (as recorded in \
             MODULE.bazel.lock, refresh with --lockfile_mode=refresh)"
        )
    );
}

#[test]
fn test_get_module_file_checksum_mismatch() {
    let server = FakeServer::with_cache();
    server.serve("/myreg/modules/foo/1.0/MODULE.bazel", &["fake"]);
    let known_files = IndexMap::from([(
        format!("{SERVER}/myreg/modules/foo/1.0/MODULE.bazel"),
        Some(sha256("original")),
    )]);
    let registry = create(
        &format!("{SERVER}/myreg"),
        LockfileMode::Update,
        &known_files,
        &[],
    );
    let e = registry
        .get_module_file(
            &create_module_key("foo", "1.0"),
            &StoredEventHandler::new(),
            &server,
        )
        .unwrap_err();
    assert_eq!(
        e,
        RegistryException::Io(format!(
            "Failed to fetch registry file {SERVER}/myreg/modules/foo/1.0/MODULE.bazel: Checksum \
             was {} but wanted {}",
            sha256("fake"),
            sha256("original")
        ))
    );
}

#[test]
fn test_get_module_file_checksum_mismatch_ignored_with_lockfile_mode_off() {
    let server = FakeServer::with_cache();
    server.serve("/myreg/modules/foo/1.0/MODULE.bazel", &["fake"]);
    let known_files = IndexMap::from([(
        format!("{SERVER}/myreg/modules/foo/1.0/MODULE.bazel"),
        Some(sha256("original")),
    )]);
    let registry = create(
        &format!("{SERVER}/myreg"),
        LockfileMode::Off,
        &known_files,
        &[],
    );
    assert_eq!(
        registry
            .get_module_file(
                &create_module_key("foo", "1.0"),
                &StoredEventHandler::new(),
                &server
            )
            .unwrap(),
        ModuleFile::create(
            "fake".as_bytes(),
            format!("{SERVER}/myreg/modules/foo/1.0/MODULE.bazel")
        )
    );
}

const REGISTRY_JSON: &str = "{\n  \"module_base_path\": \"/hello/foo\"\n}\n";
const SOURCE_JSON: &str = "{\n  \"type\": \"local_path\",\n  \"path\": \"../bar/project_x\"\n}\n";

#[test]
fn test_get_repo_spec_checksum() {
    let server = FakeServer::with_cache();
    server.serve("/bazel_registry.json", &[REGISTRY_JSON]);
    server.serve_bytes(
        "/modules/foo/1.0/source.json",
        SOURCE_JSON.as_bytes().to_vec(),
    );
    let known_files = IndexMap::from([(
        format!("{SERVER}/modules/foo/2.0/source.json"),
        Some(sha256("unused")),
    )]);
    let reporter = StoredEventHandler::new();
    let registry = create(SERVER, LockfileMode::Update, &known_files, &[]);
    assert_eq!(
        registry
            .get_repo_spec(
                &create_module_key("foo", "1.0"),
                &IndexMap::new(),
                &reporter,
                &server
            )
            .unwrap(),
        local_path_repo_spec("/hello/bar/project_x")
    );
    let mut got = recorded(&reporter);
    got.sort();
    let mut want = vec![
        (
            format!("{SERVER}/bazel_registry.json"),
            Some(sha256(REGISTRY_JSON).to_string()),
        ),
        (
            format!("{SERVER}/modules/foo/1.0/source.json"),
            Some(sha256(SOURCE_JSON).to_string()),
        ),
    ];
    want.sort();
    assert_eq!(got, want);

    let registry = create(
        SERVER,
        LockfileMode::Update,
        &recorded_checksums(&reporter),
        &[],
    );
    // The recorded hashes are used for repo cache hits even when the server content changes.
    server.unserve("/bazel_registry.json");
    server.unserve("/modules/foo/1.0/source.json");
    assert_eq!(
        registry
            .get_repo_spec(
                &create_module_key("foo", "1.0"),
                &IndexMap::new(),
                &StoredEventHandler::new(),
                &server
            )
            .unwrap(),
        local_path_repo_spec("/hello/bar/project_x")
    );
}

#[test]
fn test_get_repo_spec_checksum_mismatch() {
    let server = FakeServer::with_cache();
    server.serve_bytes("/bazel_registry.json", REGISTRY_JSON.as_bytes().to_vec());
    let malicious_source_json = SOURCE_JSON.replace("project_x", "malicious");
    server.serve_bytes(
        "/modules/foo/1.0/source.json",
        malicious_source_json.as_bytes().to_vec(),
    );
    let known_files = IndexMap::from([
        (
            format!("{SERVER}/bazel_registry.json"),
            Some(sha256(REGISTRY_JSON)),
        ),
        (
            format!("{SERVER}/modules/foo/1.0/source.json"),
            Some(sha256(SOURCE_JSON)),
        ),
    ]);
    let registry = create(SERVER, LockfileMode::Update, &known_files, &[]);
    let e = registry
        .get_repo_spec(
            &create_module_key("foo", "1.0"),
            &IndexMap::new(),
            &StoredEventHandler::new(),
            &server,
        )
        .unwrap_err();
    assert_eq!(
        e,
        RegistryException::Io(format!(
            "Failed to fetch registry file {SERVER}/modules/foo/1.0/source.json: Checksum was {} \
             but wanted {}",
            sha256(&malicious_source_json),
            sha256(SOURCE_JSON)
        ))
    );
}

#[test]
fn test_bazel_registry_checksum_mismatch() {
    let server = FakeServer::with_cache();
    let malicious_registry_json = REGISTRY_JSON.replace("foo", "malicious");
    server.serve_bytes(
        "/bazel_registry.json",
        malicious_registry_json.as_bytes().to_vec(),
    );
    server.serve_bytes(
        "/modules/foo/1.0/source.json",
        SOURCE_JSON.as_bytes().to_vec(),
    );
    let known_files = IndexMap::from([
        (
            format!("{SERVER}/bazel_registry.json"),
            Some(sha256(REGISTRY_JSON)),
        ),
        (
            format!("{SERVER}/modules/foo/1.0/source.json"),
            Some(sha256(SOURCE_JSON)),
        ),
    ]);
    let registry = create(SERVER, LockfileMode::Update, &known_files, &[]);
    let e = registry
        .get_repo_spec(
            &create_module_key("foo", "1.0"),
            &IndexMap::new(),
            &StoredEventHandler::new(),
            &server,
        )
        .unwrap_err();
    assert_eq!(
        e,
        RegistryException::Io(format!(
            "Failed to fetch registry file {SERVER}/bazel_registry.json: Checksum was {} but \
             wanted {}",
            sha256(&malicious_registry_json),
            sha256(REGISTRY_JSON)
        ))
    );
}

// RegistryFactoryTest.badSchemes
#[test]
fn bad_schemes() {
    let e = factory()
        .create_registry(
            "/home/www",
            LockfileMode::Update,
            &IndexMap::new(),
            &IndexMap::new(),
            None,
            &IndexSet::new(),
        )
        .unwrap_err();
    assert!(e.get_message().contains("Registry URL has no scheme"));
    let e = factory()
        .create_registry(
            "foo://bar",
            LockfileMode::Update,
            &IndexMap::new(),
            &IndexMap::new(),
            None,
            &IndexSet::new(),
        )
        .unwrap_err();
    assert!(
        e.get_message()
            .contains("Unrecognized registry URL protocol")
    );
}

// RegistryFactoryTest.badPath
#[test]
fn bad_path() {
    let e = factory()
        .create_registry(
            "file:c:/path/to/workspace/registry",
            LockfileMode::Update,
            &IndexMap::new(),
            &IndexMap::new(),
            None,
            &IndexSet::new(),
        )
        .unwrap_err();
    assert!(e.get_message().contains("Registry URL path is not valid"));
}

// Not in the Java tests: yanked versions from the lockfile and the default registry.
#[test]
fn try_get_yanked_versions_from_lockfile_modes() {
    let key = create_module_key("foo", "1.0");
    let source_json = format!("{SERVER}/modules/foo/1.0/source.json");
    let known = IndexMap::from([(source_json, Some(sha256("x")))]);
    let reg = create(SERVER, LockfileMode::Update, &known, &[]);
    assert_eq!(
        reg.try_get_yanked_versions_from_lockfile(&key),
        Some(blaze_bzlmod_model::YankedVersionsValue::none_yanked())
    );
    let reg = create(SERVER, LockfileMode::Refresh, &known, &[]);
    assert_eq!(reg.try_get_yanked_versions_from_lockfile(&key), None);
    let reg = create(SERVER, LockfileMode::Update, &IndexMap::new(), &[]);
    assert_eq!(reg.try_get_yanked_versions_from_lockfile(&key), None);
    assert_eq!(
        blaze_bzlmod_model::DEFAULT_REGISTRIES,
        &["https://bcr.bazel.build/"]
    );
    // Default registry URL construction: no double slash.
    let bcr = create(
        "https://bcr.bazel.build/",
        LockfileMode::Update,
        &IndexMap::new(),
        &[],
    );
    let server = FakeServer::default();
    let reporter = StoredEventHandler::new();
    let _ = bcr.get_module_file(&key, &reporter, &server);
    assert_eq!(
        recorded(&reporter),
        vec![(
            "https://bcr.bazel.build/modules/foo/1.0/MODULE.bazel".to_string(),
            None
        )]
    );
}
