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

// ---------------------------------------------------------------------------------------------
// Header checksums (review 2026-11-18 round 2, N4)
// ---------------------------------------------------------------------------------------------

/// `entry` with its checksum field replaced by `field`.
fn with_checksum_field(mut entry: Vec<u8>, field: &[u8; 8]) -> Vec<u8> {
    entry[148..156].copy_from_slice(field);
    entry
}

fn entry_named(name: &str) -> Vec<u8> {
    tar_entry(
        name,
        field(b"0000644\0"),
        field(b"00000000000\0"),
        name.as_bytes(),
    )
}

/// Commons Compress does not verify the header checksum (`TarArchiveEntry.isCheckSumOK()` is
/// only stored), so an entry with a wrong or blank checksum is extracted, and so is the entry
/// after it.
///
/// The expected results are those of commons-compress-1.26.1.jar (the version Bazel 9.3.2 pins)
/// run on archives with these checksum fields: `TarArchiveInputStream` returned the entry with
/// `isCheckSumOK() == false` and its content, then `good.txt`.
#[test]
fn entry_with_wrong_header_checksum_is_extracted() {
    for (name, checksum) in [
        ("wrong.txt", b"0000000\0"),
        ("blank.txt", &[0u8; 8]),
        ("spaces.txt", b"        "),
        // parseOctal: a field that starts with NUL is 0, whatever follows.
        ("nulfirst.txt", b"\0zzzzzzz"),
        // parseOctal trims all trailing NULs and spaces.
        ("trail.txt", b"123 \0 \0 "),
    ] {
        let (tmp, result) = extract(&[
            with_checksum_field(entry_named(name), checksum),
            entry_named("good.txt"),
        ]);
        result.unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let out = tmp.path().join("out");
        assert_eq!(
            std::fs::read(out.join(name)).unwrap(),
            name.as_bytes(),
            "{name}"
        );
        assert_eq!(std::fs::read(out.join("good.txt")).unwrap(), b"good.txt");
    }
}

/// `TarUtils.verifyCheckSum` reads the stored checksum with `parseOctal`, so a checksum field
/// that is not an octal number fails the archive with `IOException("Corrupted TAR archive.")`,
/// like any other corrupt numeric field. Results of the real jar, as above.
#[test]
fn checksum_field_that_is_not_octal_is_corrupted() {
    for checksum in [b"zzzzzzzz", b"12345678", b"12 34\0  "] {
        corrupted(&[
            with_checksum_field(entry_named("bad.txt"), checksum),
            entry_named("good.txt"),
        ]);
    }
}

// ---------------------------------------------------------------------------------------------
// The other numeric fields of every header (review 2026-11-18 round 3)
// ---------------------------------------------------------------------------------------------

fn with_field(mut entry: Vec<u8>, range: std::ops::Range<usize>, bytes: &[u8]) -> Vec<u8> {
    entry[range].copy_from_slice(bytes);
    // Keep the checksum valid, so that only the field under test differs.
    entry[148..156].copy_from_slice(b"        ");
    let sum: u32 = entry[..512].iter().map(|b| *b as u32).sum();
    entry[148..156].copy_from_slice(format!("{:06o}\0 ", sum).as_bytes());
    entry
}

/// A header of an empty file (no data blocks).
fn empty_entry(name: &str) -> Vec<u8> {
    tar_entry(name, field(b"0000644\0"), field(b"00000000000\0"), b"")
}

/// `TarUtils.parseOctalOrBinary`: a blank `size`, `uid` or `gid` is 0. Results of
/// commons-compress-1.26.1.jar on archives with these fields: the entry (empty) and then
/// `good.txt` were returned.
#[test]
fn blank_size_uid_and_gid_fields_are_zero() {
    for (name, range, bytes) in [
        ("size_nul.txt", 124..136, &[0u8; 12][..]),
        ("size_spaces.txt", 124..136, &[b' '; 12][..]),
        ("size_no_terminator.txt", 124..136, &b"000000000000"[..]),
        ("uid_nul.txt", 108..116, &[0u8; 8][..]),
        ("gid_spaces.txt", 116..124, &[b' '; 8][..]),
    ] {
        let (tmp, result) = extract(&[
            with_field(empty_entry(name), range, bytes),
            entry_named("good.txt"),
        ]);
        result.unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let out = tmp.path().join("out");
        assert_eq!(std::fs::read(out.join(name)).unwrap(), b"", "{name}");
        assert_eq!(std::fs::read(out.join("good.txt")).unwrap(), b"good.txt");
    }
}

/// A `size`, `uid` or `gid` that is not a number fails the archive with
/// `IOException("Corrupted TAR archive.")`. Results of the real jar, as above.
#[test]
fn size_uid_and_gid_fields_that_are_not_numbers_are_corrupted() {
    for (range, bytes) in [
        (124..136, &b"zzzzzzzzzzz\0"[..]),
        (108..116, &b"zzzzzzz\0"[..]),
        (116..124, &b"zzzzzzz\0"[..]),
    ] {
        corrupted(&[
            with_field(empty_entry("bad.txt"), range, bytes),
            entry_named("good.txt"),
        ]);
    }
}
