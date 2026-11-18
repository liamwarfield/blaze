//! Builds Blaze's embedded install archive (Bazel appends a zip of the install base to its
//! client binary; Blaze embeds a zstd-compressed tar with `include_bytes!`).
//!
//! Input: the oracle's install base, `$BLAZE_ORACLE_INSTALL_BASE` or
//! `$HOME/.cache/blaze/oracle-out/ur/install/<key>` with `<key>` =
//! `pins[current].install_base_key` of the repository's `UPSTREAM.json`.
//!
//! Output in `OUT_DIR`:
//! - `install.tar.zst`: every regular file of the install base except the server jar (the first
//!   file in sorted order, `A-server.jar`; Blaze is its own server) and `install_base_key`, in
//!   byte-wise sorted path order (the order of Bazel's zip), then `install_base_key` (Blaze's own
//!   key, below) last. Entries: regular files only (no directory entries, like the zip), mode as
//!   on disk, mtime 0, uid/gid 0, no user/group names.
//! - `embedded_archive.rs`: `CONTENTS` (member names in archive order, `install_base_key` last),
//!   `INSTALL_MD5`, `BUILD_LABEL` (contents of `build-label.txt`) and `SERVER_JAR`.
//!
//! Install key: like Bazel's `md5sum $(SRCS) | md5sum` (src/md5_default.sh): for each archived
//! file except `install_base_key`, in archive order, the line `<md5 hex of contents>  <path
//! relative to the install base>\n`; the key is the md5 hex of the concatenated lines.
//!
//! Rebuilds: the install base for a given key never changes (and every file in it has a far
//! future mtime, which would make a `rerun-if-changed` on it fire on every build), so the
//! script reruns only when `UPSTREAM.json` or `BLAZE_ORACLE_INSTALL_BASE` / `HOME` change.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use md5::{Digest, Md5};

/// zstd level: ~3x faster than -19 for a slightly larger archive.
const ZSTD_LEVEL: i32 = 10;

fn main() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("crates/blaze is two levels below the repository root")
        .to_path_buf();
    let upstream_json = repo_root.join("UPSTREAM.json");
    println!("cargo:rerun-if-changed={}", upstream_json.display());
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=BLAZE_ORACLE_INSTALL_BASE");
    println!("cargo:rerun-if-env-changed=HOME");

    let install_base = match std::env::var_os("BLAZE_ORACLE_INSTALL_BASE") {
        Some(p) => PathBuf::from(p),
        None => {
            let key = install_base_key_from_upstream(&upstream_json);
            let home = std::env::var_os("HOME").unwrap_or_else(|| {
                fail("HOME is not set; set BLAZE_ORACLE_INSTALL_BASE to the oracle's install base")
            });
            PathBuf::from(home)
                .join(".cache/blaze/oracle-out/ur/install")
                .join(key)
        }
    };
    if !install_base.join("install_base_key").is_file() {
        fail(&format!(
            "the oracle install base {} is missing (no install_base_key). Run the oracle once \
             (scripts/oracle) so that it extracts its install base there, or set \
             BLAZE_ORACLE_INSTALL_BASE to an extracted Bazel 9.3.2 install base.",
            install_base.display()
        ));
    }

    // Every regular file, by path relative to the install base, sorted byte-wise.
    let mut files: Vec<String> = Vec::new();
    collect_files(&install_base, &install_base, &mut files);
    files.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    files.retain(|f| f != "install_base_key");
    if files.is_empty() {
        fail(&format!("{} contains no files", install_base.display()));
    }
    // Bazel's zip starts with the server jar ("A-server.jar" sorts first).
    let server_jar = files.remove(0);
    if !server_jar.ends_with(".jar") {
        fail(&format!(
            "expected the first file of {} to be the server jar, found {}",
            install_base.display(),
            server_jar
        ));
    }
    if !files.iter().any(|f| f == "build-label.txt") {
        fail(&format!(
            "{} has no build-label.txt",
            install_base.display()
        ));
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let archive_path = out_dir.join("install.tar.zst");
    let out = fs::File::create(&archive_path)
        .unwrap_or_else(|e| fail(&format!("cannot create {}: {}", archive_path.display(), e)));
    let encoder = zstd::stream::write::Encoder::new(out, ZSTD_LEVEL)
        .unwrap_or_else(|e| fail(&format!("zstd: {}", e)));
    let mut builder = tar::Builder::new(encoder);

    let mut md5sum_lines = String::new();
    let mut build_label = String::new();
    for rel in &files {
        let path = install_base.join(rel);
        let data = fs::read(&path)
            .unwrap_or_else(|e| fail(&format!("cannot read {}: {}", path.display(), e)));
        let mode = fs::metadata(&path)
            .unwrap_or_else(|e| fail(&format!("cannot stat {}: {}", path.display(), e)))
            .permissions()
            .mode()
            & 0o7777;
        if rel == "build-label.txt" {
            build_label = String::from_utf8(data.clone())
                .unwrap_or_else(|_| fail("build-label.txt is not UTF-8"));
        }
        md5sum_lines.push_str(&format!("{}  {}\n", hex::encode(Md5::digest(&data)), rel));
        append_file(&mut builder, rel, &data, mode);
    }
    let install_md5 = hex::encode(Md5::digest(md5sum_lines.as_bytes()));
    append_file(
        &mut builder,
        "install_base_key",
        install_md5.as_bytes(),
        0o755,
    );

    let encoder = builder
        .into_inner()
        .unwrap_or_else(|e| fail(&format!("tar: {}", e)));
    encoder
        .finish()
        .unwrap_or_else(|e| fail(&format!("zstd: {}", e)))
        .flush()
        .unwrap_or_else(|e| fail(&format!("write: {}", e)));

    let mut contents = files.clone();
    contents.push("install_base_key".to_string());
    let mut generated = String::new();
    generated.push_str("// @generated by crates/blaze/build.rs\n");
    generated.push_str(&format!(
        "pub const INSTALL_MD5: &str = {:?};\n",
        install_md5
    ));
    generated.push_str(&format!(
        "pub const BUILD_LABEL: &str = {:?};\n",
        build_label
    ));
    generated.push_str(&format!("pub const SERVER_JAR: &str = {:?};\n", server_jar));
    generated.push_str("pub const CONTENTS: &[&str] = &[\n");
    for c in &contents {
        generated.push_str(&format!("    {:?},\n", c));
    }
    generated.push_str("];\n");
    let generated_path = out_dir.join("embedded_archive.rs");
    fs::write(&generated_path, generated)
        .unwrap_or_else(|e| fail(&format!("cannot write {}: {}", generated_path.display(), e)));
}

fn append_file<W: Write>(builder: &mut tar::Builder<W>, path: &str, data: &[u8], mode: u32) {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_size(data.len() as u64);
    header.set_mode(mode);
    header.set_mtime(0);
    header.set_uid(0);
    header.set_gid(0);
    builder
        .append_data(&mut header, path, data)
        .unwrap_or_else(|e| fail(&format!("tar: cannot add {}: {}", path, e)));
}

/// Collects the regular files under `dir` as paths relative to `root`. Symlinks and other
/// special files are rejected (the oracle's install base has none, and the client's extractor
/// writes regular files only).
fn collect_files(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|e| fail(&format!("cannot list {}: {}", dir.display(), e)));
    for entry in entries {
        let entry =
            entry.unwrap_or_else(|e| fail(&format!("cannot list {}: {}", dir.display(), e)));
        let path = entry.path();
        let meta = fs::symlink_metadata(&path)
            .unwrap_or_else(|e| fail(&format!("cannot stat {}: {}", path.display(), e)));
        if meta.is_dir() {
            collect_files(root, &path, out);
        } else if meta.is_file() {
            let rel = path
                .strip_prefix(root)
                .expect("path under root")
                .to_str()
                .unwrap_or_else(|| fail(&format!("non-UTF-8 path {}", path.display())))
                .to_string();
            out.push(rel);
        } else {
            fail(&format!(
                "{} is not a regular file or directory (mode {:o})",
                path.display(),
                meta.mode()
            ));
        }
    }
}

/// `pins[current].install_base_key` of UPSTREAM.json (a minimal reader: the file is small and
/// machine-written).
fn install_base_key_from_upstream(upstream_json: &Path) -> String {
    let text = fs::read_to_string(upstream_json)
        .unwrap_or_else(|e| fail(&format!("cannot read {}: {}", upstream_json.display(), e)));
    let current = string_value_after(&text, 0, "\"current\"")
        .unwrap_or_else(|| fail("UPSTREAM.json has no \"current\""));
    let pin_key = format!("\"{}\"", current);
    // The pin object: the occurrence of "<current>" followed by ':' (not the "current" value).
    let mut search = 0;
    let pin_start = loop {
        let Some(i) = text[search..].find(&pin_key).map(|i| i + search) else {
            fail(&format!("UPSTREAM.json has no pin for {}", current));
        };
        let after = text[i + pin_key.len()..].trim_start();
        if after.starts_with(':') && after[1..].trim_start().starts_with('{') {
            break i;
        }
        search = i + pin_key.len();
    };
    string_value_after(&text, pin_start, "\"install_base_key\"").unwrap_or_else(|| {
        fail(&format!(
            "UPSTREAM.json pin {} has no install_base_key",
            current
        ))
    })
}

/// The string value of the first `"key": "value"` at or after `from`.
fn string_value_after(text: &str, from: usize, key: &str) -> Option<String> {
    let i = text[from..].find(key)? + from + key.len();
    let rest = text[i..].trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn fail(message: &str) -> ! {
    eprintln!("error: blaze build.rs: {}", message);
    panic!("blaze build.rs: {}", message);
}
