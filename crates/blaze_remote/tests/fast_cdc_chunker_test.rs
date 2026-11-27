//! Port of `lib/remote/chunking/FastCdcChunkerTest.java`.

#[path = "util/java_random.rs"]
mod java_random;

use blaze_remote::chunking::{ContentDefinedChunker, FastCdcChunker, FastCdcChunkingConfig};
use blaze_remote::reapi::Digest;
use blaze_remote::util::DigestUtil;
use blaze_vfs::DigestHashFunction;
use java_random::random_bytes;
use sha2::Digest as _;

/// `DIGEST_UTIL`.
fn digest_util() -> DigestUtil {
    DigestUtil::new(DigestHashFunction::Sha256)
}

fn chunk(chunker: &FastCdcChunker, data: &[u8]) -> Vec<Digest> {
    chunker.chunk_to_digests(&mut &data[..]).unwrap()
}

fn total_size(digests: &[Digest]) -> i64 {
    digests.iter().map(|d| d.size_bytes).sum()
}

fn config_1024() -> FastCdcChunkingConfig {
    FastCdcChunkingConfig::new(1024, 2, 0)
}

#[test]
fn chunk_to_digests_empty_input_returns_empty_list() {
    let chunker = FastCdcChunker::new(digest_util());
    assert!(chunk(&chunker, &[]).is_empty());
}

#[test]
fn chunk_to_digests_small_input_returns_single_chunk() {
    let chunker = FastCdcChunker::from_config(&config_1024(), digest_util());
    let digests = chunk(&chunker, &random_bytes(42, 100));
    assert_eq!(digests.len(), 1);
    assert_eq!(digests[0].size_bytes, 100);
}

#[test]
fn chunk_to_digests_data_at_min_size_returns_single_chunk() {
    let config = config_1024();
    let chunker = FastCdcChunker::from_config(&config, digest_util());
    let digests = chunk(
        &chunker,
        &random_bytes(42, config.min_chunk_size() as usize),
    );
    assert_eq!(digests.len(), 1);
    assert_eq!(digests[0].size_bytes, i64::from(config.min_chunk_size()));
}

#[test]
fn chunk_to_digests_large_input_produces_multiple_chunks() {
    let config = config_1024();
    let chunker = FastCdcChunker::from_config(&config, digest_util());
    let data = random_bytes(42, config.max_chunk_size() as usize * 3);
    let digests = chunk(&chunker, &data);
    assert!(digests.len() > 1);
    assert_eq!(total_size(&digests), data.len() as i64);
}

#[test]
fn chunk_to_digests_same_input_produces_same_chunks() {
    let chunker = FastCdcChunker::new(digest_util());
    let data = random_bytes(123, 2 * 1024 * 1024);
    assert_eq!(chunk(&chunker, &data), chunk(&chunker, &data));
}

#[test]
fn chunk_to_digests_chunk_sizes_within_bounds() {
    let config = config_1024();
    let chunker = FastCdcChunker::from_config(&config, digest_util());
    let digests = chunk(
        &chunker,
        &random_bytes(42, config.max_chunk_size() as usize * 10),
    );
    for digest in &digests[..digests.len() - 1] {
        assert!(digest.size_bytes >= i64::from(config.min_chunk_size()));
        assert!(digest.size_bytes <= i64::from(config.max_chunk_size()));
    }
}

#[test]
fn chunk_to_digests_last_chunk_can_be_smaller_than_min() {
    let config = config_1024();
    let chunker = FastCdcChunker::from_config(&config, digest_util());
    let data_size = config.max_chunk_size() + config.min_chunk_size() / 2;
    let digests = chunk(&chunker, &random_bytes(42, data_size as usize));
    assert!(!digests.is_empty());
    assert_eq!(total_size(&digests), i64::from(data_size));
}

#[test]
fn chunk_to_digests_digests_are_correct() {
    let chunker = FastCdcChunker::from_config(&config_1024(), digest_util());
    let data = random_bytes(42, 500);
    let digests = chunk(&chunker, &data);
    assert_eq!(digests, vec![digest_util().compute(&data)]);
}

fn assert_illegal_argument(f: impl FnOnce() + std::panic::UnwindSafe) {
    let e = std::panic::catch_unwind(f).expect_err("IllegalArgumentException");
    let message = e
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(message.starts_with("IllegalArgumentException"), "{message}");
}

#[test]
fn constructor_invalid_min_size_throws() {
    assert_illegal_argument(|| {
        FastCdcChunker::with_sizes(0, 1024, 4096, 2, 0, digest_util());
    });
}

#[test]
fn constructor_avg_size_less_than_min_size_throws() {
    assert_illegal_argument(|| {
        FastCdcChunker::with_sizes(1024, 512, 4096, 2, 0, digest_util());
    });
}

#[test]
fn constructor_max_size_less_than_avg_size_throws() {
    assert_illegal_argument(|| {
        FastCdcChunker::with_sizes(256, 1024, 512, 2, 0, digest_util());
    });
}

#[test]
fn constructor_avg_size_not_power_of_two_throws() {
    assert_illegal_argument(|| {
        FastCdcChunker::with_sizes(256, 1000, 4096, 2, 0, digest_util());
    });
}

#[test]
fn constructor_invalid_normalization_throws() {
    assert_illegal_argument(|| {
        FastCdcChunker::with_sizes(256, 1024, 4096, 4, 0, digest_util());
    });
}

#[test]
fn chunk_to_digests_with_default_config() {
    let chunker = FastCdcChunker::new(digest_util());
    let data = random_bytes(42, 4 * 1024 * 1024);
    let digests = chunk(&chunker, &data);
    assert!(digests.len() > 1);
    assert_eq!(total_size(&digests), data.len() as i64);
}

#[test]
fn chunk_to_digests_test_vectors_seed0() {
    verify_test_vectors(
        0,
        &[
            (0, 19186),
            (19186, 19279),
            (38465, 17354),
            (55819, 16387),
            (72206, 19940),
            (92146, 17320),
        ],
        &[
            "0f9efa589121d5d9e9e2c4ace91337d77cae866537143f6f15a0ffd525a77c2d",
            "c7c86a165573c16448cda35c9169742e85645af42be22889f8b96b8ee0ec7cb0",
            "bc88521e28a8b4479cdea5f75aa721a24f3a0a7d0be903aa6d505c574e51e89d",
            "4b8dac2652e4685c629d2bb1ae9d4448e676b86f2e67ca0b2fff3d9580184b79",
            "c0a7062da6f2386c28e086ee0cedd5732252741269838773cff1ddb05b2df6ed",
            "7fa5b12134dc75cd2ac8dc60d3a8f3c8d22f0ee9d4cf74a4aa937e2a0d2d79a5",
        ],
    );
}

#[test]
fn chunk_to_digests_test_vectors_seed666() {
    verify_test_vectors(
        666,
        &[
            (0, 17635),
            (17635, 17334),
            (34969, 19136),
            (54105, 17467),
            (71572, 23593),
            (95165, 14301),
        ],
        &[
            "cb3a9d80a3569772d4ed331ca37ab0c862c759897b890fc1aac90a4f2ea3a407",
            "d758c6b7b0b7eef1e996f8ccd17de6c645360b03a26c35541e7581348ac08944",
            "24846aefd89e510594bae3e9d7d5ea5012067601512610fed126a3c57ba993f5",
            "efa785e1fefb49f190e665f72fd246c1442079874508c312196da1fb3040d00b",
            "a2f557bdd8d40d8faada963ad5f91ec54b10ccee7c5ae72754a65137592dc607",
            "e131100b4a7147ccad19dc63c4a2fac1f5d8b644e1373eeb6803825024234efc",
        ],
    );
}

/// Test vectors from the Remote Execution API specification (fastcdc2020_test_vectors.txt);
/// test image: "Akashita" by Toriyama Sekien (1712-1788), public domain.
fn verify_test_vectors(seed: u64, expected_chunks: &[(i64, i64)], expected_hashes: &[&str]) {
    let file_data = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/chunking/SekienAkashita.jpg"),
    )
    .unwrap();
    let chunker = FastCdcChunker::with_sizes(4096, 16384, 65535, 2, seed, digest_util());
    let digests = chunk(&chunker, &file_data);
    assert_eq!(digests.len(), expected_chunks.len());
    let mut offset = 0i64;
    for (i, digest) in digests.iter().enumerate() {
        assert_eq!((offset, digest.size_bytes), expected_chunks[i]);
        let (start, len) = expected_chunks[i];
        let chunk_data = &file_data[start as usize..(start + len) as usize];
        assert_eq!(
            hex::encode(sha2::Sha256::digest(chunk_data)),
            expected_hashes[i]
        );
        offset += digest.size_bytes;
    }
}
