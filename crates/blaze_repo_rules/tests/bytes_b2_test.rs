//! Byte strings at the repository_ctx boundaries: `read`, `file`, `execute` (argv, environment,
//! working directory, output), `getenv` / `os.environ`, `which`, `path` (`readdir`, `basename`,
//! `realpath`), `symlink`, `template`, `patch`, `download`, `download_and_extract` / `extract`
//! (tar, zip and ar entry names, `strip_prefix`, `rename_files`) and the marker file.
//!
//! Blaze-only (Java has no such test). The workspace is `tests/data/bytes_b2`; the expected
//! values (`expected_rcase.txt`, `expected_tree.txt`, `expected.marker`) were recorded from the
//! oracle, Bazel 9.3.2, building `@bytes_b2_repo//:all` in a copy of that workspace with
//! `BYTES_B2_E=é` in the client environment and
//! `--repo_env=PATH=<workspace>/tôols:<workspace>/tools:/usr/bin:/bin`.

mod common;

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::path::PathBuf;

use blaze_repo_rules::registration::RepoPrecomputedValues;
use blaze_util::string_encoding::latin1_decode;
use blaze_util::string_encoding::platform_to_internal;
use common::Scratch;
use common::run;

const DATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/bytes_b2");

/// A path below `dir` from raw bytes.
fn child(dir: &Path, rel: &[u8]) -> PathBuf {
    dir.join(OsStr::from_bytes(rel))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `<path hex> <F|D|L> <content or symlink target hex>` for every entry below `root`, sorted,
/// with an absolute symlink target below `root` written as `<repo>/<rest>`.
fn tree(root: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .as_os_str()
                .as_bytes()
                .to_vec();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            if meta.file_type().is_symlink() {
                let target = std::fs::read_link(&path).unwrap();
                let target = match target.strip_prefix(root) {
                    Ok(rest) if target.is_absolute() => {
                        [b"<repo>/".as_slice(), rest.as_os_str().as_bytes()].concat()
                    }
                    _ => target.as_os_str().as_bytes().to_vec(),
                };
                out.push(format!("{} L {}", hex(&rel), hex(&target)));
            } else if meta.is_dir() {
                out.push(format!("{} D -", hex(&rel)));
                walk(root, &path, out);
            } else {
                let content = std::fs::read(&path).unwrap();
                out.push(format!("{} F {}", hex(&rel), hex(&content)));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

#[test]
fn repository_ctx_byte_strings_match_the_oracle() {
    run(|| {
        let s = Scratch::new();
        for name in [
            "MODULE.bazel",
            "BUILD.bazel",
            "bytes_util.bzl",
            "repo.bzl",
            "src.txt",
            "tpl.txt",
            "content.patch",
            "newfile.patch",
            "archive.tar",
            "archive.zip",
            "archive.ar",
        ] {
            std::fs::copy(Path::new(DATA).join(name), s.workspace.join(name)).unwrap();
        }
        // `which("tôol")` finds the first of the two on PATH.
        for dir in ["tôols", "tools"] {
            let dir = s.workspace.join(dir);
            std::fs::create_dir_all(&dir).unwrap();
            let tool = dir.join("tôol");
            std::fs::write(&tool, "#!/bin/sh\necho tool\n").unwrap();
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let workspace = platform_to_internal(s.workspace.as_os_str()).into_owned();
        let mut env = Scratch::default_env();
        // The client environment and --repo_env hold internal strings; Rust literals are
        // Unicode, so non-ASCII ones are converted from their UTF-8 bytes.
        env.insert(
            "BYTES_B2_E".to_owned(),
            latin1_decode("é".as_bytes()).into_owned(),
        );
        env.insert(
            "PATH".to_owned(),
            format!(
                "{workspace}/{}:{workspace}/tools:/usr/bin:/bin",
                latin1_decode("tôols".as_bytes())
            ),
        );
        let ev = s.evaluator_with_env(&[], RepoPrecomputedValues::default(), env);
        let name = "+bytes_b2_repo+bytes_b2_repo";
        let dir = s
            .fetch(&ev, name)
            .unwrap_or_else(|e| panic!("{e}\n{:#?}", s.events()));

        let rcase: Vec<String> = s
            .events()
            .iter()
            .filter_map(|e| e.find("RCASE ").map(|i| e[i..].to_owned()))
            .collect();
        let expected_rcase: Vec<String> =
            std::fs::read_to_string(Path::new(DATA).join("expected_rcase.txt"))
                .unwrap()
                .lines()
                .map(str::to_owned)
                .collect();
        assert_eq!(rcase, expected_rcase);

        let expected_tree: Vec<String> =
            std::fs::read_to_string(Path::new(DATA).join("expected_tree.txt"))
                .unwrap()
                .lines()
                .filter(|l| !l.starts_with('#'))
                .map(str::to_owned)
                .collect();
        assert_eq!(tree(&dir), expected_tree);

        let marker = std::fs::read(child(
            &s.output_base.join("external"),
            format!("@{name}.marker").as_bytes(),
        ))
        .unwrap();
        let expected_marker = std::fs::read(Path::new(DATA).join("expected.marker")).unwrap();
        // The first line is the predeclared input hash; it covers the attribute values (the
        // non-ASCII `text`) and the Starlark semantics, which are Bazel's defaults here as in
        // the oracle run.
        assert_eq!(
            latin1_decode(&marker),
            latin1_decode(&expected_marker),
            "marker file"
        );
    })
}
