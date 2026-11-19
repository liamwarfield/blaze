//! PAX sparse tar entries (GNU tar `--format=pax --sparse --sparse-version=0.0|0.1|1.0`):
//! Commons Compress 1.26.1 `TarArchiveInputStream` names the entry by `GNU.sparse.name` and
//! returns the expanded content (holes as zeros), which Bazel writes out.
//!
//! Fixtures: tests/data/sparse/sparse_<version>.tar, each holding `dir/holes.bin` (200000 bytes:
//! "hello" at 65536, "world" at 199995, zeros elsewhere; sha256 deb2c1ed…). The expected content
//! is the original file: Bazel 9.3.2 extracts all three to 200000 bytes with the same
//! `hash()` (scripts/repo-gaps/compare --case sparse-tar).
// Java: com/google/devtools/build/lib/bazel/repository/decompressor/CompressedTarFunction.java:59 CompressedTarFunction.decompress

use blaze_repo_fetch::decompressor::{Decompressor, DecompressorDescriptor};
use blaze_vfs::{DigestHashFunction, Path, UnixFileSystem};
use sha2::Digest as _;

const EXPECTED_SHA256: &str = "deb2c1ed978c3cca3266b51cde933fa2c58bd66ef9f664f3295db663a55c41d1";

fn extract(version: &str) -> Vec<u8> {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let tmp = tempfile::tempdir().unwrap();
    let archive = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("tests/data/sparse/sparse_{version}.tar"));
    let out = Path::create_from_str(tmp.path().to_str().unwrap(), &fs).get_relative_str("out");
    let descriptor = DecompressorDescriptor::builder()
        .set_destination_path(out.clone())
        .set_archive_path(Path::create_from_str(archive.to_str().unwrap(), &fs))
        .build();
    Decompressor::TarFunction.decompress(&descriptor).unwrap();
    std::fs::read(tmp.path().join("out/dir/holes.bin")).unwrap()
}

fn check(version: &str) {
    let content = extract(version);
    assert_eq!(content.len(), 200000, "format {version}");
    assert_eq!(&content[65536..65541], b"hello");
    assert_eq!(&content[199995..], b"world");
    assert_eq!(hex::encode(sha2::Sha256::digest(&content)), EXPECTED_SHA256);
}

#[test]
fn pax_sparse_0_0_is_expanded() {
    check("0.0");
}

#[test]
fn pax_sparse_0_1_is_expanded_under_its_sparse_name() {
    check("0.1");
}

#[test]
fn pax_sparse_1_0_is_expanded_under_its_sparse_name() {
    check("1.0");
}
