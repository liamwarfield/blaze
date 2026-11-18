//! Makes the `tar` crate as lenient about header checksums as Commons Compress.
//!
//! Commons Compress 1.26.1 (the version Bazel 9.3.2 pins) computes
//! `TarUtils.verifyCheckSum(header)` in `TarArchiveEntry.parseTarHeaderUnwrapped` and only stores
//! the result (`TarArchiveEntry.isCheckSumOK()`). `TarArchiveInputStream.getNextTarEntry` never
//! reads it, and neither does Bazel's `CompressedTarFunction`. An entry whose header checksum is
//! wrong or blank is therefore extracted like any other. The one thing `verifyCheckSum` can do
//! to an archive is fail: it reads the stored checksum with `TarUtils.parseOctal(header, 148,
//! 8)`, whose `IllegalArgumentException` for a field that is not an octal number becomes
//! `IOException("Corrupted TAR archive.")` in `TarArchiveEntry.parseTarHeader`. (Read from the
//! class files of commons-compress-1.26.1.jar, the sources not being in the pinned tree, and
//! confirmed by running that jar on archives with each kind of checksum field.)
//!
//! The `tar` crate rejects such a header ("archive header checksum mismatch") and has no switch
//! for it, so [`LenientChecksumReader`] rewrites the checksum field of every header block to
//! the value the crate expects before the crate sees it. Nothing else in a header is changed.
//! (Review 2026-11-18 round 2, N4.)

use std::io::{self, Read};

use crate::errors::IoException;

const BLOCK_SIZE: usize = 512;

/// The `size` record of PAX extended headers, as `tar::Archive` reads it (`pax_extensions_value`
/// in tar 0.4: the first `size` record; anything unparsable gives none).
fn pax_size(extensions: &[u8]) -> Option<u64> {
    for extension in tar::PaxExtensions::new(extensions) {
        let extension = extension.ok()?;
        if extension.key() != Ok("size") {
            continue;
        }
        return extension.value().ok()?.parse::<u64>().ok();
    }
    None
}

/// A tar stream with every header's checksum field made valid.
///
/// The reader follows the archive's structure the way `tar::Archive` does (`next_entry` and
/// `next_entry_raw` in tar 0.4.46) to know which blocks are headers: a header, then the entry's
/// data padded to 512 bytes, where the data size is the header's size field unless a preceding
/// PAX local extension has a `size` record. It stops interpreting (and passes the rest of the
/// stream through unchanged) where the crate stops reading headers or fails anyway: at an
/// all-zero block, a short block, or a size it cannot parse.
///
/// The extended sparse headers of an old-GNU sparse entry (type `S`) are followed as the crate
/// follows them (`parse_sparse_header`): they are not entry headers and are passed through.
///
/// The other numeric fields `TarArchiveEntry.parseTarHeaderUnwrapped` reads for every header are
/// held to Commons Compress's rules here as well, where the crate differs (round 3):
/// - `uid` and `gid` (`parseOctalOrBinary`): a field that is not a number is
///   `IOException("Corrupted TAR archive.")`; the crate does not look at them;
/// - `size` (`parseOctalOrBinary`): not a number is the same error, a negative one is
///   `IOException("broken archive, entry with negative size")`, and a blank field is 0, which
///   the crate rejects, so the field is rewritten when the crate would not read the same value.
///
/// `mode` and `mtime` are checked by `compressed_tar::parse_tar_header_numbers`.
pub(crate) struct LenientChecksumReader<R: Read> {
    inner: R,
    /// The header block being handed out.
    block: [u8; BLOCK_SIZE],
    block_len: usize,
    block_pos: usize,
    /// Bytes of entry data (with padding) before the next header.
    data_remaining: u64,
    /// Bytes of a PAX local extension entry still to be collected into `capture`.
    capture_remaining: u64,
    capture: Option<Vec<u8>>,
    /// The contents of the PAX local extension that applies to the next file entry.
    pax_extensions: Option<Vec<u8>>,
    /// The next block is an extended sparse header of the current GNU sparse entry.
    sparse_extension_pending: bool,
    passthrough: bool,
}

impl<R: Read> LenientChecksumReader<R> {
    pub(crate) fn new(inner: R) -> Self {
        LenientChecksumReader {
            inner,
            block: [0u8; BLOCK_SIZE],
            block_len: 0,
            block_pos: 0,
            data_remaining: 0,
            capture_remaining: 0,
            capture: None,
            pax_extensions: None,
            sparse_extension_pending: false,
            passthrough: false,
        }
    }

    /// Reads the next header block into `self.block` (less than a block at end of stream).
    fn fill_block(&mut self) -> io::Result<()> {
        self.block_len = 0;
        self.block_pos = 0;
        while self.block_len < BLOCK_SIZE {
            match self.inner.read(&mut self.block[self.block_len..]) {
                Ok(0) => break,
                Ok(n) => self.block_len += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Interprets `self.block` as a header: repairs its checksum and works out how much entry
    /// data follows.
    fn process_header(&mut self) -> io::Result<()> {
        if self.block_len < BLOCK_SIZE || self.block.iter().all(|b| *b == 0) {
            // End of the archive for the crate (it does not ignore zero blocks).
            self.passthrough = true;
            return Ok(());
        }

        let corrupted = || IoException::new("Corrupted TAR archive.").into_io_error();
        // TarArchiveEntry.parseTarHeaderUnwrapped: userId, groupId, size.
        use super::compressed_tar::parse_octal_or_binary;
        parse_octal_or_binary(&self.block[108..116]).ok_or_else(corrupted)?;
        parse_octal_or_binary(&self.block[116..124]).ok_or_else(corrupted)?;
        let commons_size = parse_octal_or_binary(&self.block[124..136]).ok_or_else(corrupted)?;
        if commons_size < 0 {
            return Err(
                IoException::new("broken archive, entry with negative size").into_io_error()
            );
        }
        let crate_size = tar::Header::from_byte_slice(&self.block).entry_size().ok();
        if crate_size != Some(commons_size as u64) && commons_size < (1i64 << 33) {
            // Eleven octal digits and a NUL: the form both parsers read alike.
            self.block[124..136].copy_from_slice(format!("{commons_size:011o}\0").as_bytes());
        }

        // TarUtils.verifyCheckSum: `parseOctal(header, CHKSUM_OFFSET, CHKSUMLEN)`. Its result is
        // not used by the stream, but a field it cannot parse is a corrupted archive.
        if super::compressed_tar::parse_octal(&self.block[148..156]).is_none() {
            return Err(IoException::new("Corrupted TAR archive.").into_io_error());
        }

        // The checksum is the sum of the header's bytes with the checksum field taken as eight
        // spaces, stored as six octal digits, a NUL and a space.
        let sum: u32 = self.block[..148]
            .iter()
            .chain(&self.block[156..])
            .map(|b| *b as u32)
            .sum::<u32>()
            + 8 * 32;
        self.block[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());

        let header = tar::Header::from_byte_slice(&self.block);
        let entry_type = header.entry_type();
        let is_recognized_header = header.as_gnu().is_some() || header.as_ustar().is_some();
        let is_extension_header = entry_type.is_gnu_longname()
            || entry_type.is_gnu_longlink()
            || entry_type.is_pax_local_extensions()
            || entry_type.is_pax_global_extensions();

        let Ok(mut size) = header.entry_size() else {
            self.passthrough = true;
            return Ok(());
        };
        if !is_extension_header
            && let Some(pax_size) = self.pax_extensions.as_deref().and_then(pax_size)
        {
            size = pax_size;
        }
        let Some(padded) = size
            .checked_add(BLOCK_SIZE as u64 - 1)
            .map(|s| s & !(BLOCK_SIZE as u64 - 1))
        else {
            self.passthrough = true;
            return Ok(());
        };
        self.data_remaining = padded;

        // `Archive::next_entry`: GNU long names and long links and PAX local extensions with a
        // recognized (ustar or GNU) header describe the following entry; any other entry ends
        // the group, and the PAX extensions with it.
        let pax_local = is_recognized_header && entry_type.is_pax_local_extensions();
        let describes_next_entry = is_recognized_header
            && (entry_type.is_gnu_longname()
                || entry_type.is_gnu_longlink()
                || entry_type.is_pax_local_extensions());
        if pax_local {
            self.capture = Some(Vec::new());
            self.capture_remaining = size;
            if size == 0 {
                self.pax_extensions = self.capture.take();
            }
        } else if !describes_next_entry {
            self.pax_extensions = None;
        }

        // `Archive::parse_sparse_header`: extended sparse headers follow an old-GNU sparse
        // entry's header while `isextended` is set. (A sparse type without a GNU header fails in
        // the crate.)
        if entry_type.is_gnu_sparse()
            && let Some(gnu) = header.as_gnu()
        {
            self.sparse_extension_pending = gnu.is_extended();
        }
        Ok(())
    }
}

impl<R: Read> Read for LenientChecksumReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            // A header block that is being handed out.
            if self.block_pos < self.block_len {
                let n = buf.len().min(self.block_len - self.block_pos);
                buf[..n].copy_from_slice(&self.block[self.block_pos..self.block_pos + n]);
                self.block_pos += n;
                return Ok(n);
            }
            if self.passthrough {
                return self.inner.read(buf);
            }
            // An extended sparse header: 21 sparse descriptors, then `isextended` at offset
            // 504 (`GnuExtSparseHeader`). Not an entry header; handed out as it is.
            if self.sparse_extension_pending {
                self.fill_block()?;
                if self.block_len < BLOCK_SIZE {
                    self.passthrough = true;
                    self.sparse_extension_pending = false;
                } else {
                    self.sparse_extension_pending = self.block[504] == 1;
                }
                continue;
            }
            // Entry data and padding.
            if self.data_remaining > 0 {
                let want = buf
                    .len()
                    .min(self.data_remaining.min(usize::MAX as u64) as usize);
                let n = self.inner.read(&mut buf[..want])?;
                if n == 0 {
                    self.passthrough = true;
                    return Ok(0);
                }
                self.data_remaining -= n as u64;
                if self.capture_remaining > 0 {
                    let take = n.min(self.capture_remaining.min(usize::MAX as u64) as usize);
                    if let Some(capture) = &mut self.capture {
                        capture.extend_from_slice(&buf[..take]);
                    }
                    self.capture_remaining -= take as u64;
                    if self.capture_remaining == 0 {
                        self.pax_extensions = self.capture.take();
                    }
                }
                return Ok(n);
            }
            // At a header.
            self.fill_block()?;
            if self.block_len == 0 {
                self.passthrough = true;
                return Ok(0);
            }
            if let Err(e) = self.process_header() {
                // Nothing of the corrupted header is handed out.
                self.block_len = 0;
                return Err(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ustar header for `name` with the given type flag and size, and a valid checksum.
    fn header(name: &str, typeflag: u8, size: u64) -> [u8; 512] {
        let mut h = [0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..108].copy_from_slice(b"0000644\0");
        h[108..116].copy_from_slice(b"0000000\0");
        h[116..124].copy_from_slice(b"0000000\0");
        h[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
        h[136..148].copy_from_slice(b"00000000000\0");
        h[148..156].copy_from_slice(b"        ");
        h[156] = typeflag;
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        let sum: u32 = h.iter().map(|b| *b as u32).sum();
        h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        h
    }

    fn padded(data: &[u8]) -> Vec<u8> {
        let mut out = data.to_vec();
        out.resize(out.len() + (512 - data.len() % 512) % 512, 0);
        out
    }

    /// Reads through the reader in small, odd-sized pieces.
    fn filter(archive: &[u8]) -> Vec<u8> {
        let mut reader = LenientChecksumReader::new(archive);
        let mut out = Vec::new();
        let mut buf = [0u8; 97];
        loop {
            let n = reader.read(&mut buf).unwrap();
            if n == 0 {
                return out;
            }
            out.extend_from_slice(&buf[..n]);
        }
    }

    #[test]
    fn a_valid_archive_is_unchanged() {
        let mut archive = Vec::new();
        archive.extend_from_slice(&header("a.txt", b'0', 5));
        archive.extend_from_slice(&padded(b"hello"));
        archive.extend_from_slice(&header("dir/", b'5', 0));
        archive.extend_from_slice(&header("b.txt", b'0', 600));
        archive.extend_from_slice(&padded(&[b'x'; 600]));
        archive.extend_from_slice(&[0u8; 1024]);
        assert_eq!(filter(&archive), archive);
    }

    #[test]
    fn checksum_fields_are_repaired_and_nothing_else() {
        let good_a = header("a.txt", b'0', 5);
        let good_b = header("b.txt", b'0', 3);
        let mut bad_a = good_a;
        bad_a[148..156].copy_from_slice(b"0000000\0");
        let mut bad_b = good_b;
        bad_b[148..156].copy_from_slice(&[0u8; 8]);
        // Entry data that looks like a header is not touched.
        let mut data = header("fake", b'0', 0).to_vec();
        data[148..156].copy_from_slice(b"1234567\0");

        let mut archive = Vec::new();
        archive.extend_from_slice(&bad_a);
        archive.extend_from_slice(&padded(b"hello"));
        let mut real = header("c.bin", b'0', 512);
        real[148..156].copy_from_slice(b"        ");
        archive.extend_from_slice(&real);
        archive.extend_from_slice(&data);
        archive.extend_from_slice(&bad_b);
        archive.extend_from_slice(&padded(b"abc"));
        archive.extend_from_slice(&[0u8; 1024]);

        let mut expected = Vec::new();
        expected.extend_from_slice(&good_a);
        expected.extend_from_slice(&padded(b"hello"));
        expected.extend_from_slice(&header("c.bin", b'0', 512));
        expected.extend_from_slice(&data);
        expected.extend_from_slice(&good_b);
        expected.extend_from_slice(&padded(b"abc"));
        expected.extend_from_slice(&[0u8; 1024]);
        assert_eq!(filter(&archive), expected);
    }

    /// A PAX `size` record overrides the size field of the entry it describes, as in the crate.
    #[test]
    fn pax_size_decides_where_the_next_header_is() {
        let pax = b"13 size=1024\n";
        let mut entry = header("big.bin", b'0', 0);
        entry[148..156].copy_from_slice(b"0000000\0");
        let mut after = header("after.txt", b'0', 2);
        after[148..156].copy_from_slice(b"0000000\0");
        // The entry's data: two blocks that look like headers with bad checksums.
        let mut data = Vec::new();
        for _ in 0..2 {
            let mut fake = header("fake", b'0', 0);
            fake[148..156].copy_from_slice(b"7777777\0");
            data.extend_from_slice(&fake);
        }

        let mut archive = Vec::new();
        archive.extend_from_slice(&header("PaxHeaders/big.bin", b'x', pax.len() as u64));
        archive.extend_from_slice(&padded(pax));
        archive.extend_from_slice(&entry);
        archive.extend_from_slice(&data);
        archive.extend_from_slice(&after);
        archive.extend_from_slice(&padded(b"ok"));
        archive.extend_from_slice(&[0u8; 1024]);

        let mut expected = Vec::new();
        expected.extend_from_slice(&header("PaxHeaders/big.bin", b'x', pax.len() as u64));
        expected.extend_from_slice(&padded(pax));
        expected.extend_from_slice(&header("big.bin", b'0', 0));
        expected.extend_from_slice(&data);
        expected.extend_from_slice(&header("after.txt", b'0', 2));
        expected.extend_from_slice(&padded(b"ok"));
        expected.extend_from_slice(&[0u8; 1024]);
        assert_eq!(filter(&archive), expected);
    }

    /// `TarUtils.verifyCheckSum` parses the stored checksum with `parseOctal`; a field that is
    /// not an octal number fails the archive.
    #[test]
    fn a_checksum_field_that_is_not_octal_is_a_corrupted_archive() {
        for field in [&b"zzzzzzzz"[..], b"12345678", b"12 34\0  "] {
            let mut bad = header("a.txt", b'0', 0);
            bad[148..156].copy_from_slice(field);
            let mut archive = bad.to_vec();
            archive.extend_from_slice(&[0u8; 1024]);
            let mut reader = LenientChecksumReader::new(&archive[..]);
            let e = reader.read(&mut [0u8; 512]).unwrap_err();
            assert_eq!(
                IoException::from(e).get_message(),
                "Corrupted TAR archive.",
                "{field:?}"
            );
        }
    }

    /// An old-GNU sparse entry with one extended sparse header: the extension block is not
    /// taken for an entry header, and the header after the entry's data is still repaired.
    #[test]
    fn headers_after_a_gnu_sparse_entry_are_repaired() {
        let mut sparse = header("sparse.bin", b'S', 512);
        sparse[257..265].copy_from_slice(b"ustar  \0"); // old GNU magic and version
        sparse[482] = 1; // isextended
        let fix = |h: &mut [u8; 512]| {
            h[148..156].copy_from_slice(b"        ");
            let sum: u32 = h.iter().map(|b| *b as u32).sum();
            h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        };
        fix(&mut sparse);
        // The extension block: text that would not parse as a header, isextended = 0.
        let extension = [b'z'; 504]
            .iter()
            .copied()
            .chain([0u8; 8])
            .collect::<Vec<u8>>();
        let good_after = header("after.txt", b'0', 2);
        let mut bad_after = good_after;
        bad_after[148..156].copy_from_slice(b"0000000\0");

        let mut archive = sparse.to_vec();
        archive.extend_from_slice(&extension);
        archive.extend_from_slice(&[b'd'; 512]);
        archive.extend_from_slice(&bad_after);
        archive.extend_from_slice(&padded(b"ok"));
        archive.extend_from_slice(&[0u8; 1024]);

        let mut expected = sparse.to_vec();
        expected.extend_from_slice(&extension);
        expected.extend_from_slice(&[b'd'; 512]);
        expected.extend_from_slice(&good_after);
        expected.extend_from_slice(&padded(b"ok"));
        expected.extend_from_slice(&[0u8; 1024]);
        assert_eq!(filter(&archive), expected);
    }

    /// A blank size field is 0 for Commons Compress; the field is rewritten for the crate.
    #[test]
    fn blank_size_field_is_zero() {
        for blank in [[0u8; 12], [b' '; 12]] {
            let mut h = header("empty.txt", b'0', 0);
            h[124..136].copy_from_slice(&blank);
            let mut archive = h.to_vec();
            archive.extend_from_slice(&header("next.txt", b'0', 0));
            archive.extend_from_slice(&[0u8; 1024]);
            let mut expected = header("empty.txt", b'0', 0).to_vec();
            expected.extend_from_slice(&header("next.txt", b'0', 0));
            expected.extend_from_slice(&[0u8; 1024]);
            assert_eq!(filter(&archive), expected);
        }
    }

    #[test]
    fn size_uid_and_gid_fields_that_are_not_numbers_are_a_corrupted_archive() {
        for range in [108..116, 116..124, 124..136] {
            let mut bad = header("a.txt", b'0', 0);
            for b in &mut bad[range.clone()] {
                *b = b'z';
            }
            let mut archive = bad.to_vec();
            archive.extend_from_slice(&[0u8; 1024]);
            let mut reader = LenientChecksumReader::new(&archive[..]);
            let e = reader.read(&mut [0u8; 512]).unwrap_err();
            assert_eq!(
                IoException::from(e).get_message(),
                "Corrupted TAR archive.",
                "{range:?}"
            );
        }
    }

    #[test]
    fn nothing_after_the_end_of_archive_marker_is_touched() {
        let mut bad = header("late.txt", b'0', 0);
        bad[148..156].copy_from_slice(b"0000000\0");
        let mut archive = Vec::new();
        archive.extend_from_slice(&header("a.txt", b'0', 0));
        archive.extend_from_slice(&[0u8; 512]);
        archive.extend_from_slice(&bad);
        // A trailing partial block.
        archive.extend_from_slice(b"junk");
        assert_eq!(filter(&archive), archive);
    }
}
