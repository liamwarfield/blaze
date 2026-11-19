//! `BazelRepositoryModule.beforeCommand` as `setup::prepare_repository_functions_with` ports it
//! (review 2026-11-18, S1-2, S1-3, S1-4, S2-2, S2-3, S2-4, S2-10).
//!
//! Java has no unit test of `beforeCommand`; its behaviour is covered by shell tests
//! (`bazel_repository_cache_test.sh`, `starlark_repository_test.sh`, `bazel_module_test.sh`
//! `test_downloader_config...`, `external_integration_test.sh` `test_distdir...`,
//! `repo_contents_cache_test.sh`). The expected values below are the Java's: the messages are
//! copied from `BazelRepositoryModule.java`, `CommandEnvironment.java` and `DownloadManager.java`.

mod common;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use blaze_bzlmod_model::JavaUri;
use blaze_bzlmod_model::registry::RegistryException;
use blaze_optdefs::command::common_command_options::CommonCommandOptions;
use blaze_optdefs::misc::auth_and_tls_options::AuthAndTLSOptions;
use blaze_optdefs::misc::local_execution_options::LocalExecutionOptions;
use blaze_optdefs::misc::repository_options::RepositoryOptions;
use blaze_options::{OptionsClass, OptionsParser, PriorityCategory};
use blaze_repo_fetch::file_system_lock::{FileSystemLock, LockMode};
use blaze_repo_rules::registration::RepoPrecomputedValues;
use blaze_repo_rules::setup::{
    CommandInputs, FetchSetup, RepositoryFunctions, prepare_repository_functions_in,
    prepare_repository_functions_with,
};
use blaze_starlark::StarlarkSemantics;
use blaze_vfs::unix_file_system::UnixFileSystem;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, PathFragment};
use indexmap::IndexMap;
use sha2::Digest;

use common::Scratch;

struct Parsed {
    repo: RepositoryOptions,
    common: CommonCommandOptions,
    auth: AuthAndTLSOptions,
    local: LocalExecutionOptions,
}

fn parse(args: &[&str]) -> Parsed {
    let mut parser = OptionsParser::builder()
        .options_classes(&[
            RepositoryOptions::class(),
            CommonCommandOptions::class(),
            AuthAndTLSOptions::class(),
            LocalExecutionOptions::class(),
        ])
        .build()
        .unwrap_or_else(|e| e.throw());
    parser
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            None,
            args.iter().map(|s| s.to_string()).collect(),
        )
        .unwrap();
    Parsed {
        repo: parser.get_options().unwrap(),
        common: parser.get_options().unwrap(),
        auth: parser.get_options().unwrap(),
        local: parser.get_options().unwrap(),
    }
}

fn fs() -> FileSystemRef {
    UnixFileSystem::create(DigestHashFunction::Sha256)
}

fn env(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// What a test passes besides the options.
struct Command<'a> {
    scratch: &'a Scratch,
    working_directory: Option<&'a StdPath>,
    output_base: Option<&'a StdPath>,
    output_user_root: Option<&'a StdPath>,
    client_env: IndexMap<String, String>,
    with_local_execution_options: bool,
}

impl<'a> Command<'a> {
    fn new(scratch: &'a Scratch) -> Command<'a> {
        Command {
            scratch,
            working_directory: None,
            output_base: None,
            output_user_root: None,
            client_env: IndexMap::new(),
            with_local_execution_options: false,
        }
    }

    /// `prepare_repository_functions_with` for the given command line.
    fn prepare(&self, args: &[&str]) -> Result<RepositoryFunctions, String> {
        let parsed = parse(args);
        let setup = self.fetch_setup(&parsed);
        prepare_repository_functions_with(
            &setup,
            &CommandInputs {
                output_user_root: self.output_user_root,
                common_command_options: Some(&parsed.common),
                auth_and_tls_options: Some(&parsed.auth),
                local_execution_options: self.with_local_execution_options.then_some(&parsed.local),
            },
            fs(),
        )
    }

    fn fetch_setup<'b>(&'b self, parsed: &'b Parsed) -> FetchSetup<'b> {
        FetchSetup {
            workspace: &self.scratch.workspace,
            working_directory: self.working_directory.unwrap_or(&self.scratch.workspace),
            output_base: self.output_base.unwrap_or(&self.scratch.output_base),
            install_base: &self.scratch.install_base,
            repo_options: &parsed.repo,
            starlark_semantics: StarlarkSemantics::default(),
            client_env: self.client_env.clone(),
            repo_env: self.client_env.clone(),
            nonstrict_repo_env: self.client_env.clone(),
            precomputed: RepoPrecomputedValues::default(),
            reporter: self.scratch.events.clone(),
        }
    }
}

fn root_of(scratch: &Scratch) -> PathBuf {
    scratch.workspace.parent().unwrap().to_path_buf()
}

fn path_string(p: &Option<Path>) -> Option<String> {
    p.as_ref().map(|p| p.get_path_string().to_string())
}

fn err(result: Result<RepositoryFunctions, String>) -> String {
    match result {
        Ok(_) => panic!("expected the setup to fail"),
        Err(e) => e,
    }
}

/// A server on 127.0.0.1 that answers each connection with `body` and records the request heads.
struct Server {
    port: u16,
    requests: Arc<std::sync::Mutex<Vec<String>>>,
}

impl Server {
    fn start(body: &'static str) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = requests.clone();
        std::thread::spawn(move || {
            for socket in listener.incoming() {
                let Ok(mut socket) = socket else { return };
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    match socket.read(&mut byte) {
                        Ok(1) => head.push(byte[0]),
                        _ => break,
                    }
                }
                recorded
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&head).into_owned());
                let _ = socket.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .as_bytes(),
                );
                let _ = socket.shutdown(std::net::Shutdown::Both);
            }
        });
        Server { port, requests }
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

// ---------------------------------------------------------------------------------------------
// S1-3: the repository cache location
// ---------------------------------------------------------------------------------------------

// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:325 BazelRepositoryModule.beforeCommand
#[test]
fn default_repository_cache_is_under_the_output_user_root() {
    let s = Scratch::new();
    let root = root_of(&s);
    let output_user_root = root.join("user_root");
    // `--output_base` outside the output user root, as in `blaze --output_base=/tmp/ob build`.
    let mut command = Command::new(&s);
    command.output_user_root = Some(&output_user_root);
    let functions = command.prepare(&[]).unwrap();
    assert_eq!(
        path_string(&functions.repo_contents_cache_path),
        Some(format!(
            "{}/cache/repos/v1/contents",
            output_user_root.display()
        ))
    );
    // Not next to the output base.
    assert!(!root.join("cache").exists());
}

/// The entry points without an output user root: the default output base
/// `<output_user_root>/<md5(workspace)>` gives its parent; any other output base is refused
/// instead of putting the cache next to it.
#[test]
fn default_repository_cache_without_an_output_user_root() {
    let s = Scratch::new();
    let root = root_of(&s);
    let parsed = parse(&[]);

    // An explicit output base: no guess.
    let command = Command::new(&s);
    let e = err(prepare_repository_functions_in(
        &command.fetch_setup(&parsed),
        fs(),
    ));
    assert!(
        e.contains("set up without an output user root") && e.contains("--repository_cache="),
        "{e}"
    );
    assert!(!root.join("cache").exists());

    // The client's default output base (blaze.cc GetOutputBase: md5 of the workspace path).
    let md5 = blaze_util::fingerprint::Fingerprint::with_digest_function(
        blaze_util::fingerprint::DigestHashFunction::Md5,
    )
    .add_bytes(s.workspace.display().to_string().as_bytes())
    .hex_digest_and_reset();
    // md5("") as a check of the digest used here.
    assert_eq!(
        blaze_util::fingerprint::Fingerprint::with_digest_function(
            blaze_util::fingerprint::DigestHashFunction::Md5,
        )
        .hex_digest_and_reset(),
        "d41d8cd98f00b204e9800998ecf8427e"
    );
    let output_user_root = root.join("_bazel_user");
    let output_base = output_user_root.join(md5);
    let mut command = Command::new(&s);
    command.output_base = Some(&output_base);
    let functions = prepare_repository_functions_in(&command.fetch_setup(&parsed), fs()).unwrap();
    assert_eq!(
        path_string(&functions.repo_contents_cache_path),
        Some(format!(
            "{}/cache/repos/v1/contents",
            output_user_root.display()
        ))
    );
}

// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:715 BazelRepositoryModule.toPath
#[test]
fn relative_repository_cache_and_repo_contents_cache_are_relative_to_the_workspace() {
    let s = Scratch::new();
    let root = root_of(&s);
    let cwd = s.workspace.join("sub");
    std::fs::create_dir_all(&cwd).unwrap();
    let mut command = Command::new(&s);
    // The working directory is not what a relative path is resolved against.
    command.working_directory = Some(&cwd);

    let functions = command.prepare(&["--repository_cache=../rc"]).unwrap();
    assert_eq!(
        path_string(&functions.repo_contents_cache_path),
        Some(format!("{}/rc/contents", root.display()))
    );
    drop(functions);

    let functions = command
        .prepare(&["--repository_cache=../rc", "--repo_contents_cache=../cc"])
        .unwrap();
    assert_eq!(
        path_string(&functions.repo_contents_cache_path),
        Some(format!("{}/cc", root.display()))
    );
    drop(functions);

    // An empty path disables the caches.
    let functions = command.prepare(&["--repository_cache="]).unwrap();
    assert_eq!(path_string(&functions.repo_contents_cache_path), None);
    let functions = command
        .prepare(&["--repository_cache=../rc", "--repo_contents_cache="])
        .unwrap();
    assert_eq!(path_string(&functions.repo_contents_cache_path), None);
}

// ---------------------------------------------------------------------------------------------
// S2-4: the repo contents cache checks and its shared lock
// ---------------------------------------------------------------------------------------------

// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:380 BazelRepositoryModule.beforeCommand
#[test]
fn repo_contents_cache_inside_the_main_repo_is_refused() {
    let s = Scratch::new();
    let command = Command::new(&s);
    let e = err(command.prepare(&["--repository_cache=", "--repo_contents_cache=cc"]));
    assert_eq!(
        e,
        format!(
            "The repo contents cache [{ws}/cc] is inside the main repo [{ws}]. This can cause \
             spurious failures. Disable the repo contents cache with `--repo_contents_cache=`, \
             or specify `--repo_contents_cache=<path outside the main repo>`.\n",
            ws = s.workspace.display()
        )
    );
}

// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:366 BazelRepositoryModule.beforeCommand
#[test]
fn output_base_inside_the_repo_contents_cache_is_refused() {
    let s = Scratch::new();
    let root = root_of(&s);
    let cache = root.join("cc");
    let output_base = cache.join("ob");
    std::fs::create_dir_all(&output_base).unwrap();
    // The output base is given through a symlink: Java resolves it fully.
    let link = root.join("ob-link");
    std::os::unix::fs::symlink(&output_base, &link).unwrap();
    let mut command = Command::new(&s);
    command.output_base = Some(&link);
    let e = err(command.prepare(&["--repository_cache=", "--repo_contents_cache=../cc"]));
    assert_eq!(
        e,
        format!(
            "The output base [{}] is inside the repo contents cache [{}]. This can cause \
             spurious failures. Disable the repo contents cache with `--repo_contents_cache=`, \
             or specify `--repo_contents_cache=<path that doesn't contain the output base>`.\n",
            output_base.display(),
            cache.display()
        )
    );
}

const LOCK_PROBE: &str = "BLAZE_SETUP_TEST_LOCK_PROBE";

/// Whether another process can take the lock at `lock_path` without blocking. File system locks
/// are per process (fcntl, as the JVM's `FileChannel.lock`), so the probe is this test binary
/// run again as a child for the same test.
fn other_process_can_lock(test_name: &str, lock_path: &Path, mode: &str) -> bool {
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([test_name, "--exact", "--nocapture", "--test-threads=1"])
        .env(
            LOCK_PROBE,
            format!("{mode}:{}", lock_path.get_path_string()),
        )
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    match status.code() {
        Some(0) => true,
        Some(3) => false,
        other => panic!("lock probe failed: {other:?}"),
    }
}

/// First statement of a test that probes locks: the child side of [`other_process_can_lock`].
fn run_lock_probe_if_requested() {
    let Ok(probe) = std::env::var(LOCK_PROBE) else {
        return;
    };
    let (mode, path) = probe.split_once(':').unwrap();
    let mode = match mode {
        "exclusive" => LockMode::EXCLUSIVE,
        "shared" => LockMode::SHARED,
        other => panic!("bad mode {other}"),
    };
    let path = Path::create(PathFragment::create(path), &fs());
    std::process::exit(match FileSystemLock::try_get(&path, mode) {
        Ok(_) => 0,
        Err(_) => 3,
    });
}

// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:407 BazelRepositoryModule.beforeCommand
#[test]
fn repo_contents_cache_shared_lock_is_held_for_the_command() {
    run_lock_probe_if_requested();
    const NAME: &str = "repo_contents_cache_shared_lock_is_held_for_the_command";
    let s = Scratch::new();
    let root = root_of(&s);
    let command = Command::new(&s);
    let functions = command
        .prepare(&["--repository_cache=", "--repo_contents_cache=../cc"])
        .unwrap();
    let lock_path = Path::create(
        PathFragment::create(&format!("{}/cc/gc_lock", root.display())),
        &fs(),
    );
    assert!(lock_path.exists());
    // What the garbage collector of another server does (LocalRepoContentsCache's GC idle task
    // takes the exclusive lock): it cannot while this command runs ...
    assert!(!other_process_can_lock(NAME, &lock_path, "exclusive"));
    // ... but other commands share the cache.
    assert!(other_process_can_lock(NAME, &lock_path, "shared"));
    // PORT: Java releases it in afterCommand; here it goes with the command's services.
    drop(functions);
    assert!(other_process_can_lock(NAME, &lock_path, "exclusive"));
}

/// Two command setups in one process share the lock file (review 2026-11-18 round 2, N3): the
/// lock is a per-process record lock, so the file must stay locked until the last of them is
/// done, not until the first is.
#[test]
fn repo_contents_cache_shared_lock_outlives_the_first_of_two_setups() {
    run_lock_probe_if_requested();
    const NAME: &str = "repo_contents_cache_shared_lock_outlives_the_first_of_two_setups";
    let s = Scratch::new();
    let root = root_of(&s);
    let command = Command::new(&s);
    let args = ["--repository_cache=", "--repo_contents_cache=../cc2"];
    let first = command.prepare(&args).unwrap();
    let second = command.prepare(&args).unwrap();
    let lock_path = Path::create(
        PathFragment::create(&format!("{}/cc2/gc_lock", root.display())),
        &fs(),
    );
    assert!(!other_process_can_lock(NAME, &lock_path, "exclusive"));
    drop(first);
    assert!(!other_process_can_lock(NAME, &lock_path, "exclusive"));
    assert!(other_process_can_lock(NAME, &lock_path, "shared"));
    drop(second);
    assert!(other_process_can_lock(NAME, &lock_path, "exclusive"));
}

// ---------------------------------------------------------------------------------------------
// S1-4: --override_repository
// ---------------------------------------------------------------------------------------------

// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:700 BazelRepositoryModule.getAbsolutePath
#[test]
fn override_repository_expands_workspace_and_resolves_against_the_working_directory() {
    let s = Scratch::new();
    let cwd = s.workspace.join("sub");
    std::fs::create_dir_all(&cwd).unwrap();
    let mut command = Command::new(&s);
    command.working_directory = Some(&cwd);
    let functions = command
        .prepare(&[
            "--repository_cache=",
            "--override_repository=foo=%workspace%/third_party/foo",
            "--override_repository=bar=rel/bar",
            "--override_repository=abs=/abs/path",
            "--override_repository=gone=/x",
            "--override_repository=gone=",
        ])
        .unwrap();
    let overrides: Vec<(String, String)> = functions
        .repository_overrides
        .iter()
        .map(|(k, v)| (k.clone(), v.get_path_string().to_string()))
        .collect();
    assert_eq!(
        overrides,
        vec![
            (
                "foo".to_string(),
                format!("{}/third_party/foo", s.workspace.display())
            ),
            ("bar".to_string(), format!("{}/rel/bar", cwd.display())),
            ("abs".to_string(), "/abs/path".to_string()),
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// S1-2: --downloader_config, --distdir, .netrc, --credential_helper on both download paths
// ---------------------------------------------------------------------------------------------

fn rf_uri(uri: &str) -> blaze_repo_fetch::uri::JavaUri {
    blaze_repo_fetch::uri::JavaUri::create(uri).unwrap()
}

/// `DownloadManager.download` as a repository rule's `rctx.download` reaches it.
fn rule_download(
    functions: &RepositoryFunctions,
    url: &str,
    checksum: Option<&blaze_repo_fetch::downloader::Checksum>,
    output: &StdPath,
) -> Result<Path, blaze_repo_fetch::errors::IoException> {
    rule_download_with_headers(functions, url, &IndexMap::new(), checksum, output)
}

/// [`rule_download`] with `rctx.download(headers = ...)`.
fn rule_download_with_headers(
    functions: &RepositoryFunctions,
    url: &str,
    headers: &IndexMap<String, Vec<String>>,
    checksum: Option<&blaze_repo_fetch::downloader::Checksum>,
    output: &StdPath,
) -> Result<Path, blaze_repo_fetch::errors::IoException> {
    functions
        .services
        .download_manager
        .as_ref()
        .unwrap()
        .download(
            &[rf_uri(url)],
            headers,
            &HashMap::new(),
            checksum,
            "",
            None,
            &Path::create(PathFragment::create(&output.display().to_string()), &fs()),
            &HashMap::new(),
            "test",
            false,
        )
}

/// `DownloadManager.downloadAndReadOneUrlForBzlmod` as the registry functions reach it.
fn registry_download(
    functions: &RepositoryFunctions,
    url: &str,
) -> Result<Vec<u8>, RegistryException> {
    functions
        .bzlmod_config
        .registry_fetcher
        .download_and_read_one_url_for_bzlmod(&JavaUri::new(url).unwrap(), &IndexMap::new(), None)
}

// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:432 BazelRepositoryModule.beforeCommand
// Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadManager.java:500 DownloadManager.getRewriterBlockedAllUrlsMessage
#[test]
fn downloader_config_is_applied_to_repository_and_registry_downloads() {
    let s = Scratch::new();
    let server = Server::start("mirrored");
    s.file(
        "allow.cfg",
        &format!(
            "all_blocked_message Use the mirror\n\
             block *\n\
             allow mirror.corp.example\n\
             allow 127.0.0.1\n\
             rewrite rewritten.example/(.*) http://127.0.0.1:{}/$1\n",
            server.port
        ),
    );
    let command = Command::new(&s);
    let functions = command
        .prepare(&["--repository_cache=", "--downloader_config=allow.cfg"])
        .unwrap();
    let out = root_of(&s).join("out");
    std::fs::create_dir_all(&out).unwrap();

    // Every host that is not allowed is refused, for repository rules ...
    let e = rule_download(
        &functions,
        "https://example.com/x.txt",
        None,
        &out.join("x.txt"),
    )
    .unwrap_err();
    assert_eq!(
        e.get_message(),
        "Configured URL rewriter blocked all URLs: [https://example.com/x.txt] - Use the mirror"
    );
    // ... and for registries.
    assert_eq!(
        registry_download(&functions, "https://bcr.bazel.build/bazel_registry.json"),
        Err(RegistryException::Io(
            "Configured URL rewriter blocked all URLs: \
             [https://bcr.bazel.build/bazel_registry.json] - Use the mirror"
                .to_string()
        ))
    );
    assert!(server.requests().is_empty());

    // Rewrites apply to both.
    assert_eq!(
        registry_download(&functions, "https://rewritten.example/modules/a.json"),
        Ok(b"mirrored".to_vec())
    );
    let path = rule_download(
        &functions,
        "https://rewritten.example/archive.txt",
        None,
        &out.join("archive.txt"),
    )
    .unwrap();
    assert_eq!(std::fs::read(path.get_path_string()).unwrap(), b"mirrored");
    let requests = server.requests();
    assert!(
        requests[0].starts_with("GET /modules/a.json "),
        "{requests:?}"
    );
    assert!(requests[1].starts_with("GET /archive.txt "), "{requests:?}");
}

/// `--downloader_config` filters by the URL's host. A `Host` request header cannot point an
/// allowed URL at a blocked virtual host: `HttpURLConnection` drops a caller's `Host`
/// (`isRestrictedHeader`), so the request carries the URL's (review 2026-11-18 round 3, R3-1).
#[test]
fn downloader_config_host_filter_cannot_be_bypassed_with_a_host_header() {
    let s = Scratch::new();
    let server = Server::start("allowed");
    s.file("allow.cfg", "block *\nallow 127.0.0.1\n");
    let command = Command::new(&s);
    let functions = command
        .prepare(&["--repository_cache=", "--downloader_config=allow.cfg"])
        .unwrap();
    let out = root_of(&s).join("out");
    std::fs::create_dir_all(&out).unwrap();
    let headers: IndexMap<String, Vec<String>> =
        [("Host".to_string(), vec!["blocked.example".to_string()])]
            .into_iter()
            .collect();
    let path = rule_download_with_headers(
        &functions,
        &format!("http://127.0.0.1:{}/x.txt", server.port),
        &headers,
        None,
        &out.join("x.txt"),
    )
    .unwrap();
    assert_eq!(std::fs::read(path.get_path_string()).unwrap(), b"allowed");
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let hosts: Vec<&str> = requests[0]
        .split("\r\n")
        .filter(|l| l.to_ascii_lowercase().starts_with("host:"))
        .collect();
    assert_eq!(hosts, vec![format!("Host: 127.0.0.1:{}", server.port)]);
    assert!(!requests[0].contains("blocked.example"), "{requests:?}");
}

// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:436 BazelRepositoryModule.beforeCommand
#[test]
fn bad_downloader_config_stops_the_command() {
    let s = Scratch::new();
    let command = Command::new(&s);
    // "this config file may be required for security purposes, and the build must not proceed
    // ignoring it"
    assert_eq!(
        err(command.prepare(&["--repository_cache=", "--downloader_config=missing.cfg"])),
        format!(
            "Failed to parse downloader config: Unable to find downloader config file \
             {}/missing.cfg",
            s.workspace.display()
        )
    );
    s.file("bad.cfg", "block *\nfrobnicate example.com\n");
    assert_eq!(
        err(command.prepare(&["--repository_cache=", "--downloader_config=bad.cfg"])),
        "Failed to parse downloader config at bad.cfg:2: Unable to parse: frobnicate example.com"
    );
}

// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:484 BazelRepositoryModule.beforeCommand
#[test]
fn distdir_is_consulted_before_the_network() {
    let s = Scratch::new();
    let root = root_of(&s);
    // Relative distdirs are relative to the workspace; absolute ones are used as they are.
    let relative = s.workspace.join("dist");
    let absolute = root.join("abs-dist");
    std::fs::create_dir_all(&relative).unwrap();
    std::fs::create_dir_all(&absolute).unwrap();
    std::fs::write(relative.join("a.txt"), b"from relative distdir").unwrap();
    std::fs::write(absolute.join("b.txt"), b"from absolute distdir").unwrap();
    let command = Command::new(&s);
    let distdir_flag = format!("--distdir={}", absolute.display());
    let functions = command
        .prepare(&["--repository_cache=", "--distdir=dist", &distdir_flag])
        .unwrap();
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    for (name, content) in [
        ("a.txt", &b"from relative distdir"[..]),
        ("b.txt", &b"from absolute distdir"[..]),
    ] {
        let checksum = blaze_repo_fetch::downloader::Checksum::from_string(
            blaze_repo_fetch::cache::KeyType::SHA256,
            &hex::encode(sha2::Sha256::digest(content)),
        )
        .unwrap();
        // The host does not resolve (RFC 2606 `.invalid`): only the distdir can serve this.
        let path = rule_download(
            &functions,
            &format!("https://example.invalid/{name}"),
            Some(&checksum),
            &out.join(name),
        )
        .unwrap();
        assert_eq!(std::fs::read(path.get_path_string()).unwrap(), content);
    }
}

// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:423 BazelRepositoryModule.beforeCommand
#[test]
fn netrc_credentials_are_sent_to_registries_and_repository_downloads() {
    let s = Scratch::new();
    let root = root_of(&s);
    let server = Server::start("ok");
    let netrc = root.join("my.netrc");
    std::fs::write(&netrc, "machine 127.0.0.1 login user password secret\n").unwrap();
    let mut command = Command::new(&s);
    command.client_env = env(&[("NETRC", netrc.to_str().unwrap())]);
    let functions = command.prepare(&["--repository_cache="]).unwrap();

    assert_eq!(
        registry_download(
            &functions,
            &format!("http://127.0.0.1:{}/bazel_registry.json", server.port)
        ),
        Ok(b"ok".to_vec())
    );
    let requests = server.requests();
    // Basic base64("user:secret")
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("\r\nauthorization: basic dxnlcjpzzwnyzxq=\r\n"),
        "{requests:?}"
    );
    assert!(
        requests[0].contains("Basic dXNlcjpzZWNyZXQ="),
        "{requests:?}"
    );
}

// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:428 BazelRepositoryModule.beforeCommand
#[test]
fn unparsable_netrc_is_a_warning() {
    let s = Scratch::new();
    let root = root_of(&s);
    let netrc = root.join("bad.netrc");
    std::fs::write(&netrc, "machine\n").unwrap();
    let mut command = Command::new(&s);
    command.client_env = env(&[("NETRC", netrc.to_str().unwrap())]);
    // "If the credentials extraction failed, we're letting bazel try without credentials."
    command.prepare(&["--repository_cache="]).unwrap();
    let events = s.events();
    assert!(
        events.iter().any(|e| e.starts_with(&format!(
            "Warning: Error parsing the .netrc file: Failed to parse {}: ",
            netrc.display()
        )) && e.ends_with('.')),
        "{events:?}"
    );
}

// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:463 BazelRepositoryModule.beforeCommand
#[test]
fn credential_helper_headers_are_sent_to_registries() {
    let s = Scratch::new();
    let root = root_of(&s);
    let server = Server::start("ok");
    let helper = root.join("helper");
    std::fs::write(
        &helper,
        "#!/bin/sh\ncat > /dev/null\necho '{\"headers\":{\"Authorization\":[\"Bearer t\"]}}'\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    // The runtime's CredentialModule.beforeCommand (default --credential_helper_cache_duration).
    blaze_repo_fetch::authandtls::credentialhelper::CredentialModule::instance()
        .before_command("build", Duration::from_secs(30 * 60));
    let command = Command::new(&s);
    let functions = command
        .prepare(&[
            "--repository_cache=",
            &format!("--credential_helper={}", helper.display()),
        ])
        .unwrap();

    assert_eq!(
        registry_download(
            &functions,
            &format!("http://127.0.0.1:{}/bazel_registry.json", server.port)
        ),
        Ok(b"ok".to_vec())
    );
    let requests = server.requests();
    assert!(
        requests[0].contains("\r\nAuthorization: Bearer t\r\n"),
        "{requests:?}"
    );
}

// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:473 BazelRepositoryModule.beforeCommand
// Java: com/google/devtools/build/lib/runtime/CommandLinePathFactory.java:146 CommandLinePathFactory.create
#[test]
fn credential_helper_not_on_path_is_an_init_failure() {
    let s = Scratch::new();
    let mut command = Command::new(&s);
    command.client_env = env(&[("PATH", "/does/not/exist")]);
    // env.getReporter().handle(Event.error(e.getMessage())), then the AbruptExitException
    // "Error initializing credential helper".
    assert_eq!(
        err(command.prepare(&["--repository_cache=", "--credential_helper=no-such-helper"])),
        "Error initializing credential helper"
    );
    let events = s.events();
    assert!(
        events.iter().any(|e| e
            == "Error: Could not find file with name 'no-such-helper' on PATH '/does/not/exist'"),
        "{events:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// S2-2, S2-3: the HTTP downloader's options and non-positive values
// ---------------------------------------------------------------------------------------------

// Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:308 CommandEnvironment.<init>
#[test]
fn http_max_parallel_downloads_must_be_positive() {
    let s = Scratch::new();
    let command = Command::new(&s);
    assert_eq!(
        err(command.prepare(&["--repository_cache=", "--http_max_parallel_downloads=0"])),
        "The maximum number of parallel downloads needs to be a positive number"
    );
}

// Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:303 CommandEnvironment.<init>
// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:305 BazelRepositoryModule.beforeCommand
#[test]
fn non_positive_scales_and_negative_retries_are_ignored_with_java_s_warnings() {
    let s = Scratch::new();
    let command = Command::new(&s);
    let functions = command
        .prepare(&[
            "--repository_cache=",
            "--http_timeout_scaling=0",
            "--experimental_scale_timeouts=-1",
            "--experimental_repository_downloader_retries=-1",
        ])
        .unwrap();
    assert_eq!(functions.services.timeout_scaling, 1.0);
    assert_eq!(
        s.events(),
        vec![
            "Warning: Ignoring request to scale http timeouts by a non-positive factor".to_string(),
            "Warning: Ignoring request to scale timeouts for repositories by a non-positive \
             factor"
                .to_string(),
        ]
    );
}

/// `--http_connector_attempts` (not `--experimental_repository_downloader_retries`) is the
/// connector's attempt count: with one attempt a server error is final, with two the second
/// request succeeds.
// Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:319 CommandEnvironment.<init>
#[test]
fn http_connector_attempts_come_from_the_http_option() {
    fn serve(responses: Vec<&'static str>) -> (u16, std::thread::JoinHandle<usize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let handle = std::thread::spawn(move || {
            let mut served = 0;
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            while served < responses.len() && std::time::Instant::now() < deadline {
                let Ok((mut socket, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                socket.set_nonblocking(false).unwrap();
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    match socket.read(&mut byte) {
                        Ok(1) => head.push(byte[0]),
                        _ => break,
                    }
                }
                let _ = socket.write_all(responses[served].as_bytes());
                let _ = socket.shutdown(std::net::Shutdown::Both);
                served += 1;
            }
            served
        });
        (port, handle)
    }
    const ERROR: &str = "HTTP/1.1 500 Oops\r\nConnection: close\r\nContent-Length: 0\r\n\r\n";
    const OK: &str = "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok";

    let s = Scratch::new();
    let command = Command::new(&s);

    // One attempt; the download manager's own retries are off.
    let (port, server) = serve(vec![ERROR]);
    let functions = command
        .prepare(&[
            "--repository_cache=",
            "--http_connector_attempts=1",
            "--http_timeout_scaling=0.1",
            "--experimental_repository_downloader_retries=0",
        ])
        .unwrap();
    let result = registry_download(&functions, &format!("http://127.0.0.1:{port}/x"));
    assert!(
        matches!(result, Err(RegistryException::Io(_))),
        "{result:?}"
    );
    assert_eq!(server.join().unwrap(), 1);

    // Two attempts.
    let (port, server) = serve(vec![ERROR, OK]);
    let functions = command
        .prepare(&[
            "--repository_cache=",
            "--http_connector_attempts=2",
            "--http_timeout_scaling=0.1",
            "--experimental_repository_downloader_retries=0",
        ])
        .unwrap();
    assert_eq!(
        registry_download(&functions, &format!("http://127.0.0.1:{port}/x")),
        Ok(b"ok".to_vec())
    );
    assert_eq!(server.join().unwrap(), 2);
}

// ---------------------------------------------------------------------------------------------
// S2-10: the process wrapper's options
// ---------------------------------------------------------------------------------------------

// Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:72 ProcessWrapper.fromCommandEnvironment
#[test]
fn process_wrapper_options_come_from_local_execution_options() {
    let s = Scratch::new();
    // A command without LocalExecutionOptions: no kill delay, no graceful SIGTERM.
    let command = Command::new(&s);
    let functions = command.prepare(&["--repository_cache="]).unwrap();
    assert_eq!(functions.services.process_wrapper_kill_delay, None);
    assert!(!functions.services.process_wrapper_graceful_sigterm);

    let mut command = Command::new(&s);
    command.with_local_execution_options = true;
    // The option's default (15 seconds).
    let functions = command.prepare(&["--repository_cache="]).unwrap();
    assert_eq!(
        functions.services.process_wrapper_kill_delay,
        Some(Duration::from_secs(15))
    );
    assert!(!functions.services.process_wrapper_graceful_sigterm);

    let functions = command
        .prepare(&[
            "--repository_cache=",
            "--local_termination_grace_seconds=3",
            "--experimental_process_wrapper_graceful_sigterm",
        ])
        .unwrap();
    assert_eq!(
        functions.services.process_wrapper_kill_delay,
        Some(Duration::from_secs(3))
    );
    assert!(functions.services.process_wrapper_graceful_sigterm);
}
