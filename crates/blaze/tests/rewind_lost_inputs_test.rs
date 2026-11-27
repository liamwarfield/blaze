//! End-to-end action rewinding: a blob lost from Bazel's remote worker between two builds of one
//! server, rebuilt with `--rewind_lost_inputs` (review 2026-11-26-rewinding, S3).
//!
//! The scenario is `build_without_the_bytes_test#test_remote_cache_eviction_retries` with
//! rewinding instead of the retry: `//a:foo` runs remotely and `//a:bar` (tagged
//! `no-remote-exec`) consumes its output locally; the worker is restarted with an empty cache,
//! so `foo.out` is lost when `bar` needs it.
//!
//! Expected results recorded from the oracle (Bazel 9.3.2, 2026-11-26, the same commands):
//! exit code 0, "Build completed successfully, 2 total actions", and an execution log in which
//! `//a:foo` ran again on the remote runner (the rewound action) and `//a:bar` ran on
//! `linux-sandbox`.
//!
//! Needs Bazel's remote worker (`scripts/remote/worker`, built from the pinned source), so it is
//! ignored by default: `scripts/cargo-capped test -p blaze --test rewind_lost_inputs_test --
//! --ignored`.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn worker(args: &[&str]) {
    let status = Command::new(repo_root().join("scripts/remote/worker"))
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "scripts/remote/worker {args:?}");
}

fn wait_for_port_closed(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
        assert!(
            Instant::now() < deadline,
            "the old worker still serves port {port}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

struct Blaze {
    workspace: PathBuf,
    output_user_root: PathBuf,
}

impl Blaze {
    /// Runs one command; returns (exit code, stderr).
    fn run(&self, args: &[&str]) -> (i32, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_blaze"))
            .current_dir(&self.workspace)
            .env_clear()
            .env("HOME", std::env::var("HOME").unwrap())
            .env("PATH", "/usr/bin:/bin")
            .arg(format!(
                "--output_user_root={}",
                self.output_user_root.display()
            ))
            .args(["--max_idle_secs=60", "--nohome_rc", "--nosystem_rc"])
            .args(args)
            .output()
            .unwrap();
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }
}

/// The spawns of an execution log (`--execution_log_json_file`): (target label, runner).
fn spawns(execution_log: &Path) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(execution_log).unwrap();
    let mut spawns = Vec::new();
    for value in serde_json::Deserializer::from_str(&text).into_iter::<serde_json::Value>() {
        let value = value.unwrap();
        spawns.push((
            value["targetLabel"].as_str().unwrap_or("").to_owned(),
            value["runner"].as_str().unwrap_or("").to_owned(),
        ));
    }
    spawns
}

#[test]
#[ignore = "needs Bazel's remote worker (scripts/remote/worker); run with --ignored"]
fn lost_remote_input_is_rewound() {
    let base =
        PathBuf::from(std::env::var("HOME").unwrap()).join(".cache/blaze/work/rewinding/itest");
    let workspace = base.join("ws");
    std::fs::create_dir_all(workspace.join("a")).unwrap();
    std::fs::write(workspace.join("MODULE.bazel"), "").unwrap();
    std::fs::write(
        workspace.join("a/BUILD"),
        r#"genrule(
  name = 'foo',
  srcs = ['foo.in'],
  outs = ['foo.out'],
  cmd = 'cat $(SRCS) > $@',
)

genrule(
  name = 'bar',
  srcs = ['foo.out', 'bar.in'],
  outs = ['bar.out'],
  cmd = 'cat $(SRCS) > $@',
  tags = ['no-remote-exec'],
)
"#,
    )
    .unwrap();
    std::fs::write(workspace.join("a/foo.in"), "foo\n").unwrap();
    std::fs::write(workspace.join("a/bar.in"), "bar\n").unwrap();
    let blaze = Blaze {
        workspace: workspace.clone(),
        output_user_root: base.join("our"),
    };
    let port = free_port();
    let name = format!("rewind-itest-{port}");
    let remote = format!("--remote_executor=grpc://localhost:{port}");
    let execution_log = base.join("execlog.json");

    worker(&["start", &name, &port.to_string()]);
    let (rc, err) = blaze.run(&[
        "build",
        &remote,
        "--remote_download_minimal",
        "--jobs=4",
        "//a:bar",
    ]);
    assert_eq!(rc, 0, "{err}");
    let (rc, err) = blaze.run(&["clean"]);
    assert_eq!(rc, 0, "{err}");
    let (rc, err) = blaze.run(&[
        "build",
        &remote,
        "--remote_download_minimal",
        "--jobs=4",
        "//a:bar",
    ]);
    assert_eq!(rc, 0, "{err}");

    // A fresh worker: every blob of the first builds is gone.
    worker(&["stop", &name]);
    wait_for_port_closed(port);
    worker(&["start", &name, &port.to_string()]);
    std::fs::write(workspace.join("a/bar.in"), "updated bar\n").unwrap();
    let (rc, err) = blaze.run(&[
        "build",
        &remote,
        "--remote_download_minimal",
        "--jobs=4",
        "--rewind_lost_inputs",
        &format!("--execution_log_json_file={}", execution_log.display()),
        "//a:bar",
    ]);
    let _ = blaze.run(&["shutdown"]);
    worker(&["stop", &name]);

    assert_eq!(rc, 0, "{err}");
    assert!(
        err.contains("Build completed successfully, 2 total actions"),
        "{err}"
    );
    let spawns = spawns(&execution_log);
    assert!(
        spawns.contains(&("//a:foo".to_owned(), "remote".to_owned())),
        "the rewound //a:foo runs remotely: {spawns:?}"
    );
    assert!(
        spawns.contains(&("//a:bar".to_owned(), "linux-sandbox".to_owned())),
        "{spawns:?}"
    );
}
