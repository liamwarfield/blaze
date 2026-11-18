//! Port of `HashInputStreamTest`.

use std::io::Cursor;

use blaze_repo_fetch::cache::KeyType;
use blaze_repo_fetch::downloader::Checksum;
use blaze_repo_fetch::downloader::streams::{
    HashInputStream, InputStream, ReadInputStream, transfer_to,
};

fn read_all(checksum: &str) -> Result<Vec<u8>, blaze_repo_fetch::errors::IoException> {
    let mut s = HashInputStream::new(
        Box::new(ReadInputStream::new(Cursor::new(b"hello".to_vec()))),
        Checksum::from_string(KeyType::SHA1, checksum).unwrap(),
    );
    let mut out = Vec::new();
    let r = transfer_to(&mut s, &mut out);
    let c = s.close();
    r?;
    c?;
    Ok(out)
}

#[test]
fn valid_checksum_reads_ok() {
    assert_eq!(
        read_all("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d").unwrap(),
        b"hello"
    );
}

#[test]
fn bad_checksum_throws_io_exception() {
    let e = read_all("0000000000000000000000000000000000000000").unwrap_err();
    assert!(e.get_message().contains("Checksum"), "{}", e);
}
