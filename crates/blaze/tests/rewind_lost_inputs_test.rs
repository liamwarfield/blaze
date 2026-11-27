//! End-to-end action rewinding: a blob lost from Bazel's remote worker between two builds of one
//! server, rebuilt with `--rewind_lost_inputs` (review 2026-11-26-rewinding, S3).
//!
//! The scenario is `build_without_the_bytes_test#test_remote_cache_eviction_retries` with
//! rewinding instead of the retry: `//a:foo` runs remotely and `//a:bar` (tagged
//! `no-remote-exec`) consumes its output locally; the worker is restarted with an empty cache,
//! so `foo.out` is lost when `bar` needs it.
//!
//! The rewind build sets `--experimental_remote_cache_eviction_retries=0`, so the build cannot
//! succeed by retrying the whole build after the cache error (review round 2, RW-6).
//!
//! Expected results recorded from the oracle (Bazel 9.3.2, 2026-11-26, the same commands):
//! exit code 0, no "retrying the build" line, "Build completed successfully, 2 total actions", and an execution log in which
//! `//a:foo` ran again on the remote runner (the rewound action) and `//a:bar` ran on
//! `linux-sandbox`.
//!
//! The second test, `shared_action_copies_rewound_once` (review RW-2), loses the input of two
//! copies of one shared action: `B-shared.out` is declared by both `//shared:shared_1` and
//! `//shared:shared_2` (one shareable action), runs locally and reads `A-shared.out`, which the
//! remote worker built and forgot. Java's waiting copy restarts its node and waits for the
//! rewound input (`ActionExecutionState`, `Obsolete` → `scheduleRestart`); executing it with the
//! stale input would lose the input again and rewind `A-shared.out` a second time. Expected
//! results recorded from the oracle (Bazel 9.3.2, 2026-11-26, `rewinding/bin/rw2.sh oracle`): exit
//! code 0, "4 total actions", and in the execution log `SharedA` once on `remote` (the one
//! rewind), `SharedB` once and `SharedOut` twice on `linux-sandbox`. Whether the second copy
//! reaches the owner's state before the owner's loss is a race; the test asserts the outcome.
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
    spawn_fields(execution_log, "targetLabel")
}

/// The spawns of an execution log: (`field`, runner).
fn spawn_fields(execution_log: &Path, field: &str) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(execution_log).unwrap();
    let mut spawns = Vec::new();
    for value in serde_json::Deserializer::from_str(&text).into_iter::<serde_json::Value>() {
        let value = value.unwrap();
        spawns.push((
            value[field].as_str().unwrap_or("").to_owned(),
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
        // Without the build retry, success can only come from rewinding (review RW-6).
        "--experimental_remote_cache_eviction_retries=0",
        &format!("--execution_log_json_file={}", execution_log.display()),
        "//a:bar",
    ]);
    let _ = blaze.run(&["shutdown"]);
    worker(&["stop", &name]);

    assert_eq!(rc, 0, "{err}");
    assert!(!err.contains("retrying the build"), "{err}");
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

#[test]
#[ignore = "needs Bazel's remote worker (scripts/remote/worker); run with --ignored"]
fn shared_action_copies_rewound_once() {
    let base = PathBuf::from(std::env::var("HOME").unwrap())
        .join(".cache/blaze/work/rewinding/itest-shared");
    let workspace = base.join("ws");
    std::fs::create_dir_all(workspace.join("shared")).unwrap();
    std::fs::write(workspace.join("MODULE.bazel"), "").unwrap();
    std::fs::write(
        workspace.join("shared/shared.bzl"),
        r#"def _shared_impl(ctx):
    a = ctx.actions.declare_file("A-shared.out")
    ctx.actions.run_shell(inputs = [ctx.file.src], outputs = [a], mnemonic = "SharedA",
        progress_message = "Copying %s input to A-shared.out" % ctx.attr.name,
        command = "cp %s %s" % (ctx.file.src.path, a.path))
    b = ctx.actions.declare_file("B-shared.out")
    ctx.actions.run_shell(inputs = [a, ctx.file.b_in], outputs = [b], mnemonic = "SharedB",
        execution_requirements = {"no-remote-exec": "1"},
        progress_message = "Copying A-shared.out to B-shared.out on behalf of %s" % ctx.attr.name,
        command = "cat %s %s > %s" % (a.path, ctx.file.b_in.path, b.path))
    out = ctx.outputs.out
    ctx.actions.run_shell(inputs = [b], outputs = [out], mnemonic = "SharedOut",
        execution_requirements = {"no-remote-exec": "1"},
        command = "cp %s %s" % (b.path, out.path))
    return [DefaultInfo(files = depset([out]))]

shared = rule(implementation = _shared_impl, attrs = {
    "src": attr.label(mandatory = True, allow_single_file = True),
    "b_in": attr.label(mandatory = True, allow_single_file = True),
    "out": attr.output(mandatory = True),
})
"#,
    )
    .unwrap();
    std::fs::write(
        workspace.join("shared/BUILD"),
        r#"load("//shared:shared.bzl", "shared")
shared(name = "shared_1", src = "in.txt", b_in = "b.in", out = "shared_1.out")
shared(name = "shared_2", src = "in.txt", b_in = "b.in", out = "shared_2.out")
"#,
    )
    .unwrap();
    std::fs::write(workspace.join("shared/in.txt"), "shared input\n").unwrap();
    std::fs::write(workspace.join("shared/b.in"), "b\n").unwrap();
    let blaze = Blaze {
        workspace: workspace.clone(),
        output_user_root: base.join("our"),
    };
    let port = free_port();
    let name = format!("rewind-shared-itest-{port}");
    let remote = format!("--remote_executor=grpc://localhost:{port}");
    let execution_log = base.join("execlog.json");
    let targets = ["//shared:shared_1", "//shared:shared_2"];
    let build = |extra: &[&str]| {
        let mut args = vec![
            "build",
            remote.as_str(),
            "--remote_download_minimal",
            "--jobs=4",
        ];
        args.extend_from_slice(extra);
        args.extend_from_slice(&targets);
        blaze.run(&args)
    };

    worker(&["start", &name, &port.to_string()]);
    let (rc, err) = build(&[]);
    assert_eq!(rc, 0, "{err}");
    let (rc, err) = blaze.run(&["clean"]);
    assert_eq!(rc, 0, "{err}");
    let (rc, err) = build(&[]);
    assert_eq!(rc, 0, "{err}");

    // A fresh worker: A-shared.out is gone; b.in changes, so both copies of B run again.
    worker(&["stop", &name]);
    wait_for_port_closed(port);
    worker(&["start", &name, &port.to_string()]);
    std::fs::write(workspace.join("shared/b.in"), "updated b\n").unwrap();
    let execution_log_flag = format!("--execution_log_json_file={}", execution_log.display());
    let (rc, err) = build(&[
        "--rewind_lost_inputs",
        "--experimental_remote_cache_eviction_retries=0",
        &execution_log_flag,
    ]);
    let _ = blaze.run(&["shutdown"]);
    worker(&["stop", &name]);

    assert_eq!(rc, 0, "{err}");
    assert!(!err.contains("retrying the build"), "{err}");
    assert!(
        err.contains("Build completed successfully, 4 total actions"),
        "{err}"
    );
    let mut counts = std::collections::BTreeMap::new();
    for spawn in spawn_fields(&execution_log, "mnemonic") {
        *counts.entry(spawn).or_insert(0) += 1;
    }
    let expected: std::collections::BTreeMap<(String, String), i32> = [
        (("SharedA", "remote"), 1),
        (("SharedB", "linux-sandbox"), 1),
        (("SharedOut", "linux-sandbox"), 2),
    ]
    .into_iter()
    .map(|((m, r), n)| ((m.to_owned(), r.to_owned()), n))
    .collect();
    assert_eq!(
        counts, expected,
        "one rewind of A-shared.out, one run of the shared B"
    );
}
