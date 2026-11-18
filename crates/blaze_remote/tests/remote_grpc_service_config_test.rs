#![allow(clippy::field_reassign_with_default)]
//! Port of `src/test/java/com/google/devtools/build/lib/remote/RemoteGrpcServiceConfigTest.java`.
//!
//! Java runs the tests on an `InMemoryFileSystem` (not ported); here they run on the real file
//! system under a temporary directory, so Java's `/workspace` is `<tempdir>/workspace`.

use std::time::Duration;

use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_options::Options;
use blaze_remote::remote_grpc_service_config;
use blaze_repo_fetch::fs_utils;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};
use serde_json::{Value, json};

/// `new Scratch(new InMemoryFileSystem(DigestHashFunction.SHA256))`.
struct Scratch {
    _dir: tempfile::TempDir,
    file_system: FileSystemRef,
    root: String,
}

impl Scratch {
    fn new() -> Scratch {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        Scratch {
            _dir: dir,
            file_system: UnixFileSystem::create(DigestHashFunction::Sha256),
            root,
        }
    }

    /// `scratch.dir(path)`.
    fn dir(&self, path: &str) -> Path {
        let dir = Path::create_from_str(&format!("{}{}", self.root, path), &self.file_system);
        dir.create_directory_and_parents().unwrap();
        dir
    }

    /// `scratch.file(path, content)`.
    fn file(&self, path: &str, content: &str) {
        let file = Path::create_from_str(&format!("{}{}", self.root, path), &self.file_system);
        file.get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        fs_utils::write_content(&file, content.as_bytes()).unwrap();
    }
}

// Java: RemoteGrpcServiceConfigTest.parseRemoteOptions
fn parse_remote_options(args: &[&str]) -> RemoteOptions {
    Options::<RemoteOptions>::parse(args)
        .unwrap_or_else(|e| panic!("{}", e.get_message()))
        .into_options()
}

#[test]
fn create_uses_remote_timeout_for_remote_services() {
    let config = remote_grpc_service_config::create_from_timeout(Duration::from_secs(123));
    assert_eq!(
        Value::Object(config),
        json!({
            "methodConfig": [
                {
                    "name": [
                        {"service": "build.bazel.remote.execution.v2.ActionCache"},
                        {"service": "build.bazel.remote.execution.v2.Capabilities"},
                        {"service": "build.bazel.remote.execution.v2.ContentAddressableStorage"},
                        {"service": "google.bytestream.ByteStream"},
                        {"service": "build.bazel.remote.asset.v1.Fetch"},
                    ],
                    "timeout": "123s",
                }
            ]
        })
    );
}

#[test]
fn create_uses_user_supplied_json_file() {
    let scratch = Scratch::new();
    let workspace = scratch.dir("/workspace");
    scratch.file(
        "/workspace/service_config.json",
        r#"{
  "methodConfig": [
    {
      "name": [
        {
          "service": "build.bazel.remote.execution.v2.ActionCache",
          "method": "GetActionResult"
        },
        {"service": "google.bytestream.ByteStream"}
      ],
      "timeout": "3.500s"
    }
  ]
}
"#,
    );

    let config = remote_grpc_service_config::create(
        &parse_remote_options(&["--remote_grpc_service_config=service_config.json"]),
        &workspace,
    )
    .unwrap();

    assert_eq!(
        Value::Object(config),
        json!({
            "methodConfig": [
                {
                    "name": [
                        {
                            "service": "build.bazel.remote.execution.v2.ActionCache",
                            "method": "GetActionResult",
                        },
                        {"service": "google.bytestream.ByteStream"},
                    ],
                    "timeout": "3.500s",
                }
            ]
        })
    );
}

#[test]
fn create_rejects_unsupported_json_fields() {
    let scratch = Scratch::new();
    let workspace = scratch.dir("/workspace");
    scratch.file(
        "/workspace/service_config.json",
        r#"{
  "methodConfig": [
    {
      "name": [{"service": "google.bytestream.ByteStream"}],
      "timeout": "1s",
      "retryPolicy": {}
    }
  ]
}
"#,
    );

    let e = remote_grpc_service_config::create(
        &parse_remote_options(&["--remote_grpc_service_config=service_config.json"]),
        &workspace,
    )
    .unwrap_err();

    assert!(
        e.to_string()
            .contains("methodConfig[0] contains unsupported field 'retryPolicy'"),
        "{e}"
    );
}
