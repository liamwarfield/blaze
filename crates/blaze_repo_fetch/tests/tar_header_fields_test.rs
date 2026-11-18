//! Tests of the numeric tar header fields `CompressedTarFunction` reads (`mode`, `mtime`), with
//! hand-built headers.
//!
//! Bazel has no JUnit test for this; the expected values follow Apache Commons Compress 1.26.1
//! (`TarUtils.parseOctal`, `TarArchiveEntry.parseTarHeader`), which Bazel's
//! `CompressedTarFunction.java:147` relies on: `filePath.chmod(entry.getMode() | 0400)`, where a
//! blank mode field is 0, and a field with a byte that is not an octal digit fails with
//! `IOException("Corrupted TAR archive.")`.

use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

use blaze_repo_fetch::decompressor::{Decompressor, DecompressorDescriptor};
use blaze_repo_fetch::errors::DecompressorError;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};

fn fs() -> FileSystemRef {
    UnixFileSystem::create(DigestHashFunction::Sha256)
}

fn path_of(fs: &FileSystemRef, p: &std::path::Path) -> Path {
    Path::create_from_str(
        p.canonicalize()
            .unwrap_or(p.to_path_buf())
            .to_str()
            .unwrap(),
        fs,
    )
}

/// `s` padded with NULs to the width of a header field.
fn field<const N: usize>(s: &[u8]) -> [u8; N] {
    let mut f = [0u8; N];
    f[..s.len()].copy_from_slice(s);
    f
}

/// One USTAR entry for a regular file: the 512-byte header with the given raw `mode` and `mtime`
/// fields, then the data padded to a block.
fn tar_entry(name: &str, mode: [u8; 8], mtime: [u8; 12], data: &[u8]) -> Vec<u8> {
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    h[100..108].copy_from_slice(&mode);
    h[108..116].copy_from_slice(b"0000000\0"); // uid
    h[116..124].copy_from_slice(b"0000000\0"); // gid
    h[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes()); // size
    h[136..148].copy_from_slice(&mtime);
    h[148..156].copy_from_slice(b"        "); // chksum, as spaces while summing
    h[156] = b'0'; // typeflag: regular file
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    let sum: u32 = h.iter().map(|b| *b as u32).sum();
    h[148..156].copy_from_slice(format!("{:06o}\0 ", sum).as_bytes());
    let mut out = h.to_vec();
    out.extend_from_slice(data);
    out.resize(out.len() + (512 - data.len() % 512) % 512, 0);
    out
}

/// Writes the entries as a tar archive and extracts it with `TarFunction`.
fn extract(entries: &[Vec<u8>]) -> (tempfile::TempDir, Result<Path, DecompressorError>) {
    let tmp = tempfile::tempdir().unwrap();
    let mut archive = Vec::new();
    for e in entries {
        archive.extend_from_slice(e);
    }
    archive.extend_from_slice(&[0u8; 1024]);
    let archive_path = tmp.path().join("archive.tar");
    std::fs::write(&archive_path, archive).unwrap();
    let fs = fs();
    let descriptor = DecompressorDescriptor::builder()
        .set_destination_path(path_of(&fs, tmp.path()).get_relative_str("out"))
        .set_archive_path(path_of(&fs, &archive_path))
        .build();
    let result = Decompressor::TarFunction.decompress(&descriptor);
    (tmp, result)
}

fn permissions(tmp: &tempfile::TempDir, name: &str) -> u32 {
    let m = std::fs::metadata(tmp.path().join("out").join(name)).unwrap();
    m.permissions().mode() & 0o7777
}

fn mtime_seconds(tmp: &tempfile::TempDir, name: &str) -> i64 {
    std::fs::metadata(tmp.path().join("out").join(name))
        .unwrap()
        .mtime()
}

fn corrupted(entries: &[Vec<u8>]) {
    let (_tmp, result) = extract(entries);
    match result {
        Err(DecompressorError::Io(e)) => assert_eq!(e.get_message(), "Corrupted TAR archive."),
        Err(other) => panic!("expected IOException, got {:?}", other),
        Ok(_) => panic!("expected IOException, but the archive was extracted"),
    }
}

#[test]
fn blank_mode_field_of_nuls_is_zero() {
    let (tmp, result) = extract(&[tar_entry(
        "a.txt",
        [0u8; 8],
        field(b"00000000012\0"),
        b"hello",
    )]);
    result.unwrap();
    assert_eq!(
        std::fs::read(tmp.path().join("out/a.txt")).unwrap(),
        b"hello"
    );
    // entry.getMode() | 0400 with getMode() == 0.
    assert_eq!(permissions(&tmp, "a.txt"), 0o400);
    assert_eq!(mtime_seconds(&tmp, "a.txt"), 0o12);
}

#[test]
fn blank_mode_field_of_spaces_is_zero() {
    let (tmp, result) = extract(&[
        tar_entry("a.txt", *b"        ", field(b"00000000012\0"), b"hello"),
        tar_entry("b.txt", *b"       \0", field(b"00000000012\0"), b"hello"),
    ]);
    result.unwrap();
    assert_eq!(permissions(&tmp, "a.txt"), 0o400);
    assert_eq!(permissions(&tmp, "b.txt"), 0o400);
}

#[test]
fn mode_field_skips_leading_spaces_and_trailing_nuls_and_spaces() {
    let (tmp, result) = extract(&[
        tar_entry("a.txt", *b"0000755\0", field(b"00000000012\0"), b"hello"),
        tar_entry("b.txt", *b"   644 \0", field(b"00000000012\0"), b"hello"),
        tar_entry(
            "c.txt",
            *b"640\0\0\0\0\0",
            field(b"00000000012\0"),
            b"hello",
        ),
    ]);
    result.unwrap();
    assert_eq!(permissions(&tmp, "a.txt"), 0o755);
    assert_eq!(permissions(&tmp, "b.txt"), 0o644);
    assert_eq!(permissions(&tmp, "c.txt"), 0o640);
}

#[test]
fn blank_mtime_field_is_zero() {
    let (tmp, result) = extract(&[
        tar_entry("a.txt", *b"0000644\0", [0u8; 12], b"hello"),
        tar_entry("b.txt", *b"0000644\0", *b"            ", b"hello"),
    ]);
    result.unwrap();
    assert_eq!(permissions(&tmp, "a.txt"), 0o644);
    assert_eq!(mtime_seconds(&tmp, "a.txt"), 0);
    assert_eq!(mtime_seconds(&tmp, "b.txt"), 0);
}

#[test]
fn mode_field_with_a_byte_that_is_not_an_octal_digit_is_a_corrupted_archive() {
    corrupted(&[tar_entry(
        "a.txt",
        *b"00006x4\0",
        field(b"00000000012\0"),
        b"hello",
    )]);
    corrupted(&[tar_entry(
        "a.txt",
        *b"0000648\0",
        field(b"00000000012\0"),
        b"hello",
    )]);
}

#[test]
fn mode_field_with_an_embedded_nul_is_a_corrupted_archive() {
    // Only trailing NULs and spaces are trimmed; a NUL before a digit is an invalid byte.
    corrupted(&[tar_entry(
        "a.txt",
        *b"644\0 755",
        field(b"00000000012\0"),
        b"hello",
    )]);
}

#[test]
fn mode_field_with_a_sign_is_a_corrupted_archive() {
    corrupted(&[tar_entry(
        "a.txt",
        *b"+000644\0",
        field(b"00000000012\0"),
        b"hello",
    )]);
}

#[test]
fn mtime_field_with_a_byte_that_is_not_an_octal_digit_is_a_corrupted_archive() {
    corrupted(&[tar_entry(
        "a.txt",
        *b"0000644\0",
        field(b"0000000001z\0"),
        b"hello",
    )]);
}
