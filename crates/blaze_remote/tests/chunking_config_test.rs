//! Port of `lib/remote/chunking/ChunkingConfigTest.java`.

#[path = "util/java_random.rs"]
mod java_random;

use blaze_remote::chunking::{
    ChunkingConfig, ContentDefinedChunker, FastCdcChunker, FastCdcChunkingConfig, RepMaxCdcChunker,
    RepMaxCdcChunkingConfig,
};
use blaze_remote::reapi::{
    CacheCapabilities, FastCdc2020Params, RepMaxCdcParams, ServerCapabilities, chunking_function,
};
use blaze_remote::util::DigestUtil;
use blaze_vfs::DigestHashFunction;

fn capabilities_with_fast_cdc_params(params: FastCdc2020Params) -> ServerCapabilities {
    ServerCapabilities {
        cache_capabilities: Some(CacheCapabilities {
            fast_cdc_2020_params: Some(params),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn capabilities_with_rep_max_cdc_params(params: RepMaxCdcParams) -> ServerCapabilities {
    ServerCapabilities {
        cache_capabilities: Some(CacheCapabilities {
            rep_max_cdc_params: Some(params),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn fast_cdc_params(avg_chunk_size_bytes: u64, seed: u32) -> FastCdc2020Params {
    FastCdc2020Params {
        avg_chunk_size_bytes,
        seed,
    }
}

fn rep_max_cdc_params(min_chunk_size_bytes: u64, horizon_size_bytes: u64) -> RepMaxCdcParams {
    RepMaxCdcParams {
        min_chunk_size_bytes,
        horizon_size_bytes,
    }
}

const FAST_CDC_2020: chunking_function::Value = chunking_function::Value::FastCdc2020;
const REP_MAX_CDC: chunking_function::Value = chunking_function::Value::RepMaxCdc;

#[test]
fn fast_cdc_defaults_returns_expected_values() {
    let config = FastCdcChunkingConfig::defaults();
    assert_eq!(config.chunking_function(), FAST_CDC_2020);
    assert_eq!(config.avg_chunk_size, 512 * 1024);
    assert_eq!(config.normalization_level, 2);
    assert_eq!(config.seed, 0);
    assert_eq!(
        ChunkingConfig::FastCdc(config).chunking_threshold(),
        512 * 1024 * 4
    );
}

#[test]
fn fast_cdc_min_chunk_size_returns_quarter_of_avg() {
    assert_eq!(FastCdcChunkingConfig::new(1024, 2, 0).min_chunk_size(), 256);
}

#[test]
fn fast_cdc_max_chunk_size_returns_four_times_avg() {
    assert_eq!(
        FastCdcChunkingConfig::new(1024, 2, 0).max_chunk_size(),
        4096
    );
}

#[test]
fn fast_cdc_chunking_threshold_equals_max_chunk_size() {
    let config = FastCdcChunkingConfig::new(1024, 2, 0);
    assert_eq!(
        ChunkingConfig::FastCdc(config).chunking_threshold(),
        i64::from(config.max_chunk_size())
    );
}

#[test]
fn fast_cdc_min_and_max_chunk_size_with_default_config() {
    let config = FastCdcChunkingConfig::defaults();
    assert_eq!(config.min_chunk_size(), 128 * 1024);
    assert_eq!(config.max_chunk_size(), 2048 * 1024);
}

/// PORT: `isInstanceOf(FastCdcChunker.class)` on the returned interface becomes "chunks as a
/// `FastCdcChunker` of the same configuration does".
#[test]
fn fast_cdc_new_chunker_returns_fast_cdc_chunker() {
    let digest_util = DigestUtil::new(DigestHashFunction::Sha256);
    let config = FastCdcChunkingConfig::new(1024, 2, 0);
    let data = java_random::random_bytes(42, 20000);
    let chunker = ChunkingConfig::FastCdc(config).new_chunker(digest_util.clone());
    let expected = FastCdcChunker::from_config(&config, digest_util)
        .chunk_to_digests(&mut &data[..])
        .unwrap();
    assert_eq!(chunker.chunk_to_digests(&mut &data[..]).unwrap(), expected);
}

#[test]
fn rep_max_cdc_defaults_returns_expected_values() {
    let config = RepMaxCdcChunkingConfig::defaults();
    assert_eq!(config.chunking_function(), REP_MAX_CDC);
    assert_eq!(config.min_chunk_size, 256 * 1024);
    assert_eq!(config.horizon_size, 8 * 256 * 1024);
    assert_eq!(
        ChunkingConfig::RepMaxCdc(config).chunking_threshold(),
        2 * 256 * 1024 - 1
    );
}

#[test]
fn rep_max_cdc_max_chunk_size_is_below_twice_min_chunk_size() {
    assert_eq!(
        RepMaxCdcChunkingConfig::new(1024, 8 * 1024).max_chunk_size(),
        2047
    );
}

#[test]
fn rep_max_cdc_chunking_threshold_equals_max_chunk_size() {
    let config = RepMaxCdcChunkingConfig::new(1024, 8 * 1024);
    assert_eq!(
        ChunkingConfig::RepMaxCdc(config).chunking_threshold(),
        i64::from(config.max_chunk_size())
    );
}

/// PORT: as `fast_cdc_new_chunker_returns_fast_cdc_chunker`.
#[test]
fn rep_max_cdc_new_chunker_returns_rep_max_cdc_chunker() {
    let digest_util = DigestUtil::new(DigestHashFunction::Sha256);
    let config = RepMaxCdcChunkingConfig::new(1024, 8 * 1024);
    let data = java_random::random_bytes(42, 20000);
    let chunker = ChunkingConfig::RepMaxCdc(config).new_chunker(digest_util.clone());
    let expected = RepMaxCdcChunker::from_config(&config, digest_util)
        .chunk_to_digests(&mut &data[..])
        .unwrap();
    assert_eq!(chunker.chunk_to_digests(&mut &data[..]).unwrap(), expected);
}

#[test]
fn from_server_capabilities_negotiation_prefers_fast_cdc_when_both_advertised() {
    let capabilities = ServerCapabilities {
        cache_capabilities: Some(CacheCapabilities {
            fast_cdc_2020_params: Some(fast_cdc_params(512 * 1024, 0)),
            rep_max_cdc_params: Some(rep_max_cdc_params(256 * 1024, 0)),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(matches!(
        ChunkingConfig::from_server_capabilities(&capabilities),
        Some(ChunkingConfig::FastCdc(_))
    ));
}

#[test]
fn from_server_capabilities_negotiation_uses_rep_max_cdc_when_only_it_is_advertised() {
    let capabilities = capabilities_with_rep_max_cdc_params(rep_max_cdc_params(256 * 1024, 0));
    assert!(matches!(
        ChunkingConfig::from_server_capabilities(&capabilities),
        Some(ChunkingConfig::RepMaxCdc(_))
    ));
}

#[test]
fn from_server_capabilities_negotiation_uses_fast_cdc_when_only_it_is_advertised() {
    let capabilities = capabilities_with_fast_cdc_params(fast_cdc_params(512 * 1024, 0));
    assert!(matches!(
        ChunkingConfig::from_server_capabilities(&capabilities),
        Some(ChunkingConfig::FastCdc(_))
    ));
}

#[test]
fn from_server_capabilities_negotiation_neither_advertised_returns_null() {
    let capabilities = ServerCapabilities {
        cache_capabilities: Some(CacheCapabilities::default()),
        ..Default::default()
    };
    assert_eq!(
        ChunkingConfig::from_server_capabilities(&capabilities),
        None
    );
    assert_eq!(
        ChunkingConfig::from_server_capabilities(&ServerCapabilities::default()),
        None
    );
}

#[test]
fn from_server_capabilities_without_cache_capabilities_returns_null() {
    let capabilities = ServerCapabilities::default();
    assert_eq!(
        ChunkingConfig::from_server_capabilities_for(&capabilities, FAST_CDC_2020),
        None
    );
    assert_eq!(
        ChunkingConfig::from_server_capabilities_for(&capabilities, REP_MAX_CDC),
        None
    );
}

#[test]
fn from_server_capabilities_without_params_for_requested_function_returns_null() {
    let capabilities = ServerCapabilities {
        cache_capabilities: Some(CacheCapabilities::default()),
        ..Default::default()
    };
    assert_eq!(
        ChunkingConfig::from_server_capabilities_for(&capabilities, FAST_CDC_2020),
        None
    );
    assert_eq!(
        ChunkingConfig::from_server_capabilities_for(&capabilities, REP_MAX_CDC),
        None
    );
}

#[test]
fn from_server_capabilities_fast_cdc_requested_but_only_rep_max_cdc_advertised_returns_null() {
    let capabilities = capabilities_with_rep_max_cdc_params(rep_max_cdc_params(256 * 1024, 0));
    assert_eq!(
        ChunkingConfig::from_server_capabilities_for(&capabilities, FAST_CDC_2020),
        None
    );
}

#[test]
fn from_server_capabilities_rep_max_cdc_requested_but_only_fast_cdc_advertised_returns_null() {
    let capabilities = capabilities_with_fast_cdc_params(fast_cdc_params(512 * 1024, 0));
    assert_eq!(
        ChunkingConfig::from_server_capabilities_for(&capabilities, REP_MAX_CDC),
        None
    );
}

#[test]
fn from_server_capabilities_with_fast_cdc_params_returns_config() {
    let capabilities = capabilities_with_fast_cdc_params(fast_cdc_params(256 * 1024, 42));
    let Some(ChunkingConfig::FastCdc(config)) =
        ChunkingConfig::from_server_capabilities_for(&capabilities, FAST_CDC_2020)
    else {
        panic!("not a FastCdcChunkingConfig");
    };
    assert_eq!(config.avg_chunk_size, 256 * 1024);
    assert_eq!(config.seed, 42);
    assert_eq!(
        ChunkingConfig::FastCdc(config).chunking_threshold(),
        256 * 1024 * 4
    );
}

#[test]
fn from_server_capabilities_with_default_fast_cdc_params_returns_defaults() {
    let capabilities = capabilities_with_fast_cdc_params(fast_cdc_params(512 * 1024, 0));
    assert_eq!(
        ChunkingConfig::from_server_capabilities_for(&capabilities, FAST_CDC_2020),
        Some(ChunkingConfig::FastCdc(FastCdcChunkingConfig::defaults()))
    );
}

#[test]
fn from_server_capabilities_non_power_of_two_avg_size_falls_back_to_default() {
    let capabilities = capabilities_with_fast_cdc_params(fast_cdc_params(300 * 1024, 0));
    let Some(ChunkingConfig::FastCdc(config)) =
        ChunkingConfig::from_server_capabilities_for(&capabilities, FAST_CDC_2020)
    else {
        panic!("not a FastCdcChunkingConfig");
    };
    assert_eq!(
        config.avg_chunk_size,
        FastCdcChunkingConfig::DEFAULT_AVG_CHUNK_SIZE
    );
}

#[test]
fn from_server_capabilities_with_rep_max_cdc_params_returns_config() {
    let capabilities =
        capabilities_with_rep_max_cdc_params(rep_max_cdc_params(128 * 1024, 1024 * 1024));
    let Some(ChunkingConfig::RepMaxCdc(config)) =
        ChunkingConfig::from_server_capabilities_for(&capabilities, REP_MAX_CDC)
    else {
        panic!("not a RepMaxCdcChunkingConfig");
    };
    assert_eq!(config.min_chunk_size, 128 * 1024);
    assert_eq!(config.horizon_size, 1024 * 1024);
    assert_eq!(
        ChunkingConfig::RepMaxCdc(config).chunking_threshold(),
        2 * 128 * 1024 - 1
    );
}

#[test]
fn from_server_capabilities_rep_max_cdc_min_size_out_of_range_falls_back_to_default() {
    let capabilities = capabilities_with_rep_max_cdc_params(rep_max_cdc_params(64, 1024 * 1024));
    let Some(ChunkingConfig::RepMaxCdc(config)) =
        ChunkingConfig::from_server_capabilities_for(&capabilities, REP_MAX_CDC)
    else {
        panic!("not a RepMaxCdcChunkingConfig");
    };
    assert_eq!(
        config.min_chunk_size,
        RepMaxCdcChunkingConfig::DEFAULT_MIN_CHUNK_SIZE
    );
    assert_eq!(config.horizon_size, 1024 * 1024);
}

#[test]
fn from_server_capabilities_rep_max_cdc_horizon_size_out_of_range_falls_back_to_default() {
    let capabilities =
        capabilities_with_rep_max_cdc_params(rep_max_cdc_params(128 * 1024, 1024 * 1024 * 1024));
    let Some(ChunkingConfig::RepMaxCdc(config)) =
        ChunkingConfig::from_server_capabilities_for(&capabilities, REP_MAX_CDC)
    else {
        panic!("not a RepMaxCdcChunkingConfig");
    };
    assert_eq!(config.min_chunk_size, 128 * 1024);
    assert_eq!(
        config.horizon_size,
        RepMaxCdcChunkingConfig::DEFAULT_HORIZON_SIZE_FACTOR * 128 * 1024
    );
}

#[test]
fn from_server_capabilities_rep_max_cdc_zero_horizon_size_is_accepted() {
    let capabilities = capabilities_with_rep_max_cdc_params(rep_max_cdc_params(128 * 1024, 0));
    let Some(ChunkingConfig::RepMaxCdc(config)) =
        ChunkingConfig::from_server_capabilities_for(&capabilities, REP_MAX_CDC)
    else {
        panic!("not a RepMaxCdcChunkingConfig");
    };
    assert_eq!(config.horizon_size, 0);
}
