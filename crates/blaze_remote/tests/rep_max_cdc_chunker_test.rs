//! Port of `lib/remote/chunking/RepMaxCdcChunkerTest.java`.

#[path = "util/java_random.rs"]
mod java_random;

use blaze_remote::chunking::{ContentDefinedChunker, RepMaxCdcChunker, RepMaxCdcChunkingConfig};
use blaze_remote::reapi::Digest;
use blaze_remote::util::DigestUtil;
use blaze_vfs::DigestHashFunction;
use java_random::{JavaRandom, random_bytes};
use sha2::Digest as _;

/// `DIGEST_UTIL`.
fn digest_util() -> DigestUtil {
    DigestUtil::new(DigestHashFunction::Sha256)
}

fn chunk(chunker: &RepMaxCdcChunker, data: &[u8]) -> Vec<Digest> {
    chunker.chunk_to_digests(&mut &data[..]).unwrap()
}

fn total_size(digests: &[Digest]) -> i64 {
    digests.iter().map(|d| d.size_bytes).sum()
}

#[test]
fn chunk_to_digests_empty_input_returns_empty_list() {
    let chunker = RepMaxCdcChunker::new(digest_util());
    assert!(chunk(&chunker, &[]).is_empty());
}

#[test]
fn chunk_to_digests_small_input_returns_single_chunk() {
    let chunker = RepMaxCdcChunker::with_sizes(1024, 8 * 1024, digest_util());
    let digests = chunk(&chunker, &random_bytes(42, 100));
    assert_eq!(digests.len(), 1);
    assert_eq!(digests[0].size_bytes, 100);
}

#[test]
fn chunk_to_digests_data_at_min_size_returns_single_chunk() {
    let chunker = RepMaxCdcChunker::with_sizes(1024, 8 * 1024, digest_util());
    let digests = chunk(&chunker, &random_bytes(42, 1024));
    assert_eq!(digests.len(), 1);
    assert_eq!(digests[0].size_bytes, 1024);
}

#[test]
fn chunk_to_digests_large_input_produces_multiple_chunks() {
    let config = RepMaxCdcChunkingConfig::new(1024, 8 * 1024);
    let chunker = RepMaxCdcChunker::from_config(&config, digest_util());
    let data = random_bytes(42, config.max_chunk_size() as usize * 3);
    let digests = chunk(&chunker, &data);
    assert!(digests.len() > 1);
    assert_eq!(total_size(&digests), data.len() as i64);
}

#[test]
fn chunk_to_digests_same_input_produces_same_chunks() {
    let chunker = RepMaxCdcChunker::new(digest_util());
    let data = random_bytes(123, 2 * 1024 * 1024);
    assert_eq!(chunk(&chunker, &data), chunk(&chunker, &data));
}

#[test]
fn chunk_to_digests_chunk_sizes_within_bounds() {
    let min_size = 1024;
    let chunker = RepMaxCdcChunker::with_sizes(min_size, 8 * 1024, digest_util());
    let data = random_bytes(42, min_size as usize * 50);
    let digests = chunk(&chunker, &data);
    // All chunks, including the last one, are within [minSize, 2 * minSize) because the input is
    // a multiple of minSize.
    for digest in &digests {
        assert!(digest.size_bytes >= i64::from(min_size));
        assert!(digest.size_bytes < 2 * i64::from(min_size));
    }
    assert_eq!(total_size(&digests), data.len() as i64);
}

#[test]
fn chunk_to_digests_last_chunk_is_at_least_min_size() {
    let min_size = 1024;
    let chunker = RepMaxCdcChunker::with_sizes(min_size, 8 * 1024, digest_util());
    // An input that is not a multiple of the minimum chunk size.
    let data_size = min_size * 10 + min_size / 2;
    let digests = chunk(&chunker, &random_bytes(42, data_size as usize));
    // Unlike FastCDC, RepMaxCDC guarantees that all chunks of an input of at least minSize are
    // at least minSize in size, including the last one.
    for digest in &digests {
        assert!(digest.size_bytes >= i64::from(min_size));
        assert!(digest.size_bytes < 2 * i64::from(min_size));
    }
    assert_eq!(total_size(&digests), i64::from(data_size));
}

#[test]
fn chunk_to_digests_zero_horizon_produces_uniform_chunks() {
    let min_size = 1024;
    let chunker = RepMaxCdcChunker::with_sizes(min_size, 0, digest_util());
    let digests = chunk(&chunker, &random_bytes(42, min_size as usize * 10));
    // A zero horizon degenerates into uniform chunking of minSize.
    assert_eq!(digests.len(), 10);
    for digest in &digests {
        assert_eq!(digest.size_bytes, i64::from(min_size));
    }
}

#[test]
fn chunk_to_digests_digests_are_correct() {
    let chunker = RepMaxCdcChunker::with_sizes(1024, 8 * 1024, digest_util());
    let data = random_bytes(42, 500);
    assert_eq!(chunk(&chunker, &data), vec![digest_util().compute(&data)]);
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
fn constructor_min_size_below_gear_hash_window_throws() {
    assert_illegal_argument(|| {
        RepMaxCdcChunker::with_sizes(63, 8 * 1024, digest_util());
    });
}

#[test]
fn constructor_negative_horizon_size_throws() {
    assert_illegal_argument(|| {
        RepMaxCdcChunker::with_sizes(1024, -1, digest_util());
    });
}

#[test]
fn constructor_peek_size_overflows_throws() {
    assert_illegal_argument(|| {
        RepMaxCdcChunker::with_sizes(1024 * 1024, i32::MAX / 2, digest_util());
    });
}

#[test]
fn chunk_to_digests_with_default_config() {
    let chunker = RepMaxCdcChunker::new(digest_util());
    let data = random_bytes(42, 4 * 1024 * 1024);
    let digests = chunk(&chunker, &data);
    assert!(digests.len() > 1);
    assert_eq!(total_size(&digests), data.len() as i64);
}

#[test]
fn chunk_to_digests_random_data_produces_valid_chunks() {
    let mut random = JavaRandom::new(42);
    let params = [
        (64, 0),
        (64, 256),
        (128, 1024),
        (1024, 0),
        (1024, 8 * 1024),
        (2048, 16 * 1024),
    ];
    for (min_size, horizon_size) in params {
        let peek_size = 2 * min_size + horizon_size;
        let sizes = [
            0,
            1,
            min_size - 1,
            min_size,
            min_size + 1,
            2 * min_size - 1,
            2 * min_size,
            2 * min_size + 1,
            peek_size - 1,
            peek_size,
            peek_size + 1,
            // Large enough to force multiple refills of the chunker's read buffer.
            4 * peek_size + 17,
            64 * 1024 + 1,
        ];
        for size in sizes {
            let mut data = vec![0u8; size as usize];
            random.next_bytes(&mut data);
            assert_valid_chunks(min_size, horizon_size, &data);
        }
    }
}

#[test]
fn chunk_to_digests_low_entropy_data_produces_valid_chunks() {
    // Constant and periodic inputs starve the algorithm of new hash maxima, exercising the chunk
    // completion and forced-cut paths of the implementation.
    for min_size in [64, 1024] {
        for horizon_size in [0, 8 * min_size] {
            let constant = vec![0u8; 64 * 1024 + 3];
            assert_valid_chunks(min_size, horizon_size, &constant);
            let periodic: Vec<u8> = (0..64 * 1024 + 3).map(|i| (i % 7) as u8).collect();
            assert_valid_chunks(min_size, horizon_size, &periodic);
        }
    }
}

#[test]
fn chunk_to_digests_random_parameters_produces_valid_chunks() {
    // Random small parameters maximize the rate of forced cuts and cutting point recomputation,
    // covering the rare paths of the implementation.
    let mut random = JavaRandom::new(987654321);
    for _ in 0..200 {
        let min_size = 64 + random.next_int(256);
        let horizon_size = random.next_int(4 * 1024);
        let mut data = vec![0u8; random.next_int(32 * 1024) as usize];
        random.next_bytes(&mut data);
        assert_valid_chunks(min_size, horizon_size, &data);
    }
}

#[test]
fn chunk_to_digests_large_input_produces_valid_chunks() {
    let data = random_bytes(123, 4 * 1024 * 1024);
    assert_valid_chunks(
        /* min_size= */ 1024,
        /* horizon_size= */ 8 * 1024,
        &data,
    );
}

/// Chunks `data` and verifies the guarantees of the algorithm: all chunks are within `[minSize,
/// 2 * minSize)`, except that the final chunk may be smaller than `minSize` only if the entire
/// input is; the chunk sizes add up to the input size; and chunking is deterministic.
fn assert_valid_chunks(min_size: i32, horizon_size: i32, data: &[u8]) {
    let chunker = RepMaxCdcChunker::with_sizes(min_size, horizon_size, digest_util());
    let digests = chunk(&chunker, data);
    let context = format!(
        "minSize={min_size}, horizonSize={horizon_size}, inputSize={}",
        data.len()
    );
    let mut total_size = 0i64;
    for (i, digest) in digests.iter().enumerate() {
        let size = digest.size_bytes;
        if i < digests.len() - 1 || data.len() >= min_size as usize {
            assert!(size >= i64::from(min_size), "{context}");
        }
        assert!(size < 2 * i64::from(min_size), "{context}");
        total_size += size;
    }
    assert_eq!(total_size, data.len() as i64, "{context}");
    assert_eq!(chunk(&chunker, data), digests, "{context}");
}

#[test]
fn chunk_to_digests_test_vectors() {
    verify_test_vectors(
        /* min_size= */ 4096,
        /* horizon_size= */ 32768,
        &[
            (0, 7026),
            (7026, 5198),
            (12224, 5310),
            (17534, 4271),
            (21805, 5278),
            (27083, 5588),
            (32671, 5017),
            (37688, 6761),
            (44449, 5541),
            (49990, 7612),
            (57602, 5501),
            (63103, 4695),
            (67798, 4362),
            (72160, 6221),
            (78381, 7190),
            (85571, 5858),
            (91429, 4382),
            (95811, 8022),
            (103833, 5633),
        ],
        &[
            "3c386537b8c3200dd2ab6623f6096a576d07844a4a0d5d2994d13e24887dae46",
            "cc7fb08694b471e3ba660b700b347775d1e7a55db8f308c176e2eb6776b5222c",
            "096a3e0e42f0a8b7a5a3a337e9b5b4c7fc59abcd6f69486fb68fe4c5c156300f",
            "3f4a0b00f9a04959806c9ce9e530fa5d761930ef6c8477117b58eb93196342e8",
            "d0c2f82664d8887bd2d289a75baea1d5688e6ee586f8767149bfef9c6df8fe63",
            "558e6b58c659f7463c846a0d18c155a86520d1ca69102f102635c08e8b0897e2",
            "c3c66478eb8e1632768e6e13dc31e492ac78befca38e9db85525b4b274c5b5d7",
            "a5ce22ca3a02323451ddca6db5319c26b5fc1abc5621a033c98cd3de154cd29d",
            "ec4752962eda083bf78b24cb087e07451ced85d8c0d719f32c474f7dae8fd764",
            "3d9e8dc0a5f14dee7db5eabf99bb636352081423f122a84fdb160cecc2ad01bb",
            "f8c747a292f7fd6433945356ee314b978b423ac4c3d248c8fdf36c0693eb6006",
            "71249e1031dc237ab6393381a30a10ef90d9a93f532c135b11cc0df60a767b5f",
            "95c42ec307be0ce1efde6de814c668f05a31887fde9501b478faca225cf9b8c7",
            "8869f4871a0f3fb63775e0fb7f10e170a59b71a6fa0a91a18fa3cf5fff987eba",
            "d2f91644457863f716071e76e9b1046408820aa95b36471d5e70ee5c91b611a2",
            "3507024624db4ff48ee546e984d7c60461d6a526aba7e4aa33833b162785e79e",
            "9b19e98bc5d9b6f486c7c7281fd61044b8e5333363048b20ce3dcb2ee0172980",
            "e56dbfa2a82695fed3c565857dbc644af91c983b55c13fe20d9e03bb89e009e2",
            "7d76fde9af9a911c4e580db9227e5109c5de28101f3f26637c527e80215aad38",
        ],
    );
}

#[test]
fn chunk_to_digests_test_vectors_zero_horizon() {
    verify_test_vectors(
        /* min_size= */ 8192,
        /* horizon_size= */ 0,
        &[
            (0, 8192),
            (8192, 8192),
            (16384, 8192),
            (24576, 8192),
            (32768, 8192),
            (40960, 8192),
            (49152, 8192),
            (57344, 8192),
            (65536, 8192),
            (73728, 8192),
            (81920, 8192),
            (90112, 8192),
            (98304, 11162),
        ],
        &[
            "716c5e702f4bee5f18426da9e9eb77d22aa936486741768b86fff472bc18c363",
            "33df8556634b77338abee984190d1aa21efb10411edeae3820fd61a93e489f58",
            "a05018146657a5c999868a9e3f6e94da715c519f929005ca50404eb5a30caf1f",
            "e124b2b4e8847def80e1b8da2fbfa5b483c3bcab6cf2d88e50d598d6c2d2e65c",
            "7855d8367f097e33d0d3afbdba0e17749309a0dfb48f3d373bdf94b9b8f378e1",
            "782a8f7b5367f177b94da16dc14026a3af3dcfc25b6b53f2060eafee0bc16922",
            "3be28a5d9d67d5f8f9b30fbe742406cfefb46c139b48a47eeb98837fbb55128b",
            "e6d7615aeefa30776bd0de14ad0e9ac5a738a767c63aa8c3310aa85c49d16d44",
            "316e9ca063049a490d8f8431f109f2eaf0bf74739b4c3f4be9da46144733c327",
            "2b4a043bd77df955a9fa30358597a90bca046474917e91d3715205f9d674a3bf",
            "78abb22658ea5642440139f1bf86382d4bca2d33858fd8e15a170f1c1ac19651",
            "076758af278b0fb9fd0313db8f6b796639c23241e2c8d831503e272e069d9aea",
            "e776b8d90b880e10e4fdc4f99ba3b0bfbe471f26362007a1775d4cab46a539c7",
        ],
    );
}

/// Test vectors generated with the Go reference implementation of RepMaxCDC
/// (buildbarn/go-cdc); test image: "Akashita" by Toriyama Sekien (1712-1788), public domain.
fn verify_test_vectors(
    min_size: i32,
    horizon_size: i32,
    expected_chunks: &[(i64, i64)],
    expected_hashes: &[&str],
) {
    let file_data = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/chunking/SekienAkashita.jpg"),
    )
    .unwrap();
    let chunker = RepMaxCdcChunker::with_sizes(min_size, horizon_size, digest_util());
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
