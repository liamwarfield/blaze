//! Port of `lib/remote/chunking/`: content-defined chunking (`--experimental_remote_cache_chunking`)
//! with the chunking functions of the Remote Execution API, FastCDC 2020 and RepMaxCDC.

pub mod fast_cdc_chunker;
pub mod gear_table;
pub mod rep_max_cdc_chunker;

use std::io;

use crate::reapi::{Digest, ServerCapabilities, chunking_function};
use crate::util::DigestUtil;

pub use fast_cdc_chunker::FastCdcChunker;
pub use rep_max_cdc_chunker::RepMaxCdcChunker;

/// Splits a blob into content-defined chunks.
// Java: com/google/devtools/build/lib/remote/chunking/ContentDefinedChunker.java:23 ContentDefinedChunker
pub trait ContentDefinedChunker: Send + Sync {
    /// Chunks a blob and returns the chunk digests in order.
    ///
    /// This method is used for building MerkleTree entries for large files and for chunked
    /// uploads. It only returns the content digests; callers that need the raw chunk data can
    /// read it from the original (seekable) file using the chunk sizes, similar to how whole
    /// blobs work.
    ///
    /// Chunking is deterministic: the same input always produces the same chunks for a given
    /// configuration. Fails only if reading from `input` fails; chunking and digest computation
    /// themselves do not perform any I/O.
    // Java: com/google/devtools/build/lib/remote/chunking/ContentDefinedChunker.java:38 ContentDefinedChunker.chunkToDigests
    fn chunk_to_digests(&self, input: &mut dyn io::Read) -> io::Result<Vec<Digest>>;
}

/// `java.io.InputStream.read(buf, off, len)` until `buf` is full or the end of the input:
/// returns the number of bytes read; `Interrupted` reads are retried as Java's stream never
/// reports them.
pub(crate) fn read_fully(input: &mut dyn io::Read, buf: &mut [u8]) -> io::Result<(usize, bool)> {
    let mut filled = 0;
    while filled < buf.len() {
        match input.read(&mut buf[filled..]) {
            Ok(0) => return Ok((filled, true)),
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok((filled, false))
}

/// Configuration for content-defined chunking. All sizes are in bytes.
///
/// Each variant corresponds to one of the chunking functions defined by the Remote Execution API
/// and carries the parameters negotiated from the server capabilities.
// Java: com/google/devtools/build/lib/remote/chunking/ChunkingConfig.java:29 ChunkingConfig
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkingConfig {
    FastCdc(FastCdcChunkingConfig),
    RepMaxCdc(RepMaxCdcChunkingConfig),
}

impl ChunkingConfig {
    /// The chunking function this configuration applies to.
    // Java: com/google/devtools/build/lib/remote/chunking/ChunkingConfig.java:32 ChunkingConfig.chunkingFunction
    pub fn chunking_function(&self) -> chunking_function::Value {
        match self {
            ChunkingConfig::FastCdc(c) => c.chunking_function(),
            ChunkingConfig::RepMaxCdc(c) => c.chunking_function(),
        }
    }

    /// The minimum size of a chunk. Only the last chunk of a blob may be smaller.
    // Java: com/google/devtools/build/lib/remote/chunking/ChunkingConfig.java:35 ChunkingConfig.minChunkSize
    pub fn min_chunk_size(&self) -> i32 {
        match self {
            ChunkingConfig::FastCdc(c) => c.min_chunk_size(),
            ChunkingConfig::RepMaxCdc(c) => c.min_chunk_size,
        }
    }

    /// The maximum size of a chunk.
    // Java: com/google/devtools/build/lib/remote/chunking/ChunkingConfig.java:38 ChunkingConfig.maxChunkSize
    pub fn max_chunk_size(&self) -> i32 {
        match self {
            ChunkingConfig::FastCdc(c) => c.max_chunk_size(),
            ChunkingConfig::RepMaxCdc(c) => c.max_chunk_size(),
        }
    }

    /// Blobs larger than this should be chunked. Always equal to [`Self::max_chunk_size`]:
    /// anything above it splits into at least two chunks, while anything at or below it would
    /// come back as a single chunk, making chunking pointless.
    // Java: com/google/devtools/build/lib/remote/chunking/ChunkingConfig.java:45 ChunkingConfig.chunkingThreshold
    pub fn chunking_threshold(&self) -> i64 {
        i64::from(self.max_chunk_size())
    }

    /// Creates a chunker that splits blobs according to this configuration.
    // Java: com/google/devtools/build/lib/remote/chunking/ChunkingConfig.java:50 ChunkingConfig.newChunker
    pub fn new_chunker(&self, digest_util: DigestUtil) -> Box<dyn ContentDefinedChunker> {
        match self {
            ChunkingConfig::FastCdc(c) => Box::new(FastCdcChunker::from_config(c, digest_util)),
            ChunkingConfig::RepMaxCdc(c) => Box::new(RepMaxCdcChunker::from_config(c, digest_util)),
        }
    }

    /// Returns a configuration negotiated from the server capabilities: FastCDC 2020 if the
    /// server advertises it, otherwise RepMaxCDC, or `None` if the server advertises neither.
    ///
    /// FastCDC 2020 is preferred for backward compatibility, so that a fleet of clients with
    /// mixed versions keeps producing identical chunks against the same server.
    // Java: com/google/devtools/build/lib/remote/chunking/ChunkingConfig.java:60 ChunkingConfig.fromServerCapabilities
    pub fn from_server_capabilities(capabilities: &ServerCapabilities) -> Option<ChunkingConfig> {
        Self::from_server_capabilities_for(capabilities, chunking_function::Value::FastCdc2020)
            .or_else(|| {
                Self::from_server_capabilities_for(
                    capabilities,
                    chunking_function::Value::RepMaxCdc,
                )
            })
    }

    /// Returns the configuration for the given chunking function based on the parameters
    /// advertised by the server, or `None` if the server does not support the function.
    ///
    /// Advertised parameters that are out of the expected range are replaced with defaults.
    // Java: com/google/devtools/build/lib/remote/chunking/ChunkingConfig.java:76 ChunkingConfig.fromServerCapabilities
    pub fn from_server_capabilities_for(
        capabilities: &ServerCapabilities,
        chunking_function: chunking_function::Value,
    ) -> Option<ChunkingConfig> {
        let cache_cap = capabilities.cache_capabilities.as_ref()?;
        match chunking_function {
            chunking_function::Value::FastCdc2020 => cache_cap
                .fast_cdc_2020_params
                .as_ref()
                .map(|p| ChunkingConfig::FastCdc(FastCdcChunkingConfig::from_params(p))),
            chunking_function::Value::RepMaxCdc => cache_cap
                .rep_max_cdc_params
                .as_ref()
                .map(|p| ChunkingConfig::RepMaxCdc(RepMaxCdcChunkingConfig::from_params(p))),
            _ => None,
        }
    }
}

/// Configuration for the FastCDC 2020 chunking function. All sizes are in bytes.
// Java: com/google/devtools/build/lib/remote/chunking/FastCdcChunkingConfig.java:22 FastCdcChunkingConfig
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FastCdcChunkingConfig {
    pub avg_chunk_size: i32,
    pub normalization_level: i32,
    pub seed: i32,
}

impl FastCdcChunkingConfig {
    pub const DEFAULT_AVG_CHUNK_SIZE: i32 = 512 * 1024;
    pub const DEFAULT_NORMALIZATION_LEVEL: i32 = 2;
    pub const DEFAULT_SEED: i32 = 0;

    pub fn new(avg_chunk_size: i32, normalization_level: i32, seed: i32) -> Self {
        FastCdcChunkingConfig {
            avg_chunk_size,
            normalization_level,
            seed,
        }
    }

    // Java: com/google/devtools/build/lib/remote/chunking/FastCdcChunkingConfig.java:30 FastCdcChunkingConfig.chunkingFunction
    pub fn chunking_function(&self) -> chunking_function::Value {
        chunking_function::Value::FastCdc2020
    }

    // Java: com/google/devtools/build/lib/remote/chunking/FastCdcChunkingConfig.java:35 FastCdcChunkingConfig.minChunkSize
    pub fn min_chunk_size(&self) -> i32 {
        self.avg_chunk_size / 4
    }

    // Java: com/google/devtools/build/lib/remote/chunking/FastCdcChunkingConfig.java:40 FastCdcChunkingConfig.maxChunkSize
    pub fn max_chunk_size(&self) -> i32 {
        self.avg_chunk_size.wrapping_mul(4)
    }

    // Java: com/google/devtools/build/lib/remote/chunking/FastCdcChunkingConfig.java:49 FastCdcChunkingConfig.defaults
    pub fn defaults() -> Self {
        Self::new(
            Self::DEFAULT_AVG_CHUNK_SIZE,
            Self::DEFAULT_NORMALIZATION_LEVEL,
            Self::DEFAULT_SEED,
        )
    }

    /// Creates a configuration from the parameters advertised by the server, replacing values
    /// outside the expected range with defaults.
    // Java: com/google/devtools/build/lib/remote/chunking/FastCdcChunkingConfig.java:58 FastCdcChunkingConfig.fromParams
    pub fn from_params(params: &crate::reapi::FastCdc2020Params) -> Self {
        let mut avg_size = Self::DEFAULT_AVG_CHUNK_SIZE;
        // getAvgChunkSizeBytes() is a Java long: a uint64 above Long.MAX_VALUE is negative.
        let config_avg_size = params.avg_chunk_size_bytes as i64;
        if (1024..=1024 * 1024).contains(&config_avg_size)
            && (config_avg_size & (config_avg_size - 1)) == 0
        {
            avg_size = config_avg_size as i32;
        }
        // getSeed() is a Java int.
        let seed = params.seed as i32;
        Self::new(avg_size, Self::DEFAULT_NORMALIZATION_LEVEL, seed)
    }
}

/// Configuration for the RepMaxCDC chunking function. All sizes are in bytes.
///
/// RepMaxCDC produces chunks within `[minChunkSize, 2 * minChunkSize)` in size. The horizon size
/// controls the lookahead window used to find optimal cutting points; larger values improve
/// deduplication quality with diminishing returns, and a value of zero produces uniform chunks
/// of `minChunkSize`.
// Java: com/google/devtools/build/lib/remote/chunking/RepMaxCdcChunkingConfig.java:29 RepMaxCdcChunkingConfig
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepMaxCdcChunkingConfig {
    pub min_chunk_size: i32,
    pub horizon_size: i32,
}

impl RepMaxCdcChunkingConfig {
    pub const DEFAULT_MIN_CHUNK_SIZE: i32 = 256 * 1024;
    /// The default horizon size, expressed as a multiple of the minimum chunk size.
    pub const DEFAULT_HORIZON_SIZE_FACTOR: i32 = 8;

    pub fn new(min_chunk_size: i32, horizon_size: i32) -> Self {
        RepMaxCdcChunkingConfig {
            min_chunk_size,
            horizon_size,
        }
    }

    // Java: com/google/devtools/build/lib/remote/chunking/RepMaxCdcChunkingConfig.java:37 RepMaxCdcChunkingConfig.chunkingFunction
    pub fn chunking_function(&self) -> chunking_function::Value {
        chunking_function::Value::RepMaxCdc
    }

    /// Chunks are strictly smaller than twice the minimum chunk size.
    // Java: com/google/devtools/build/lib/remote/chunking/RepMaxCdcChunkingConfig.java:42 RepMaxCdcChunkingConfig.maxChunkSize
    pub fn max_chunk_size(&self) -> i32 {
        self.min_chunk_size.wrapping_mul(2).wrapping_sub(1)
    }

    // Java: com/google/devtools/build/lib/remote/chunking/RepMaxCdcChunkingConfig.java:52 RepMaxCdcChunkingConfig.defaults
    pub fn defaults() -> Self {
        Self::new(
            Self::DEFAULT_MIN_CHUNK_SIZE,
            Self::DEFAULT_HORIZON_SIZE_FACTOR * Self::DEFAULT_MIN_CHUNK_SIZE,
        )
    }

    /// Creates a configuration from the parameters advertised by the server, replacing values
    /// outside the expected range with defaults.
    // Java: com/google/devtools/build/lib/remote/chunking/RepMaxCdcChunkingConfig.java:61 RepMaxCdcChunkingConfig.fromParams
    pub fn from_params(params: &crate::reapi::RepMaxCdcParams) -> Self {
        let mut min_size = Self::DEFAULT_MIN_CHUNK_SIZE;
        let config_min_size = params.min_chunk_size_bytes as i64;
        if (1024..=1024 * 1024).contains(&config_min_size) {
            min_size = config_min_size as i32;
        }
        let mut horizon_size = Self::DEFAULT_HORIZON_SIZE_FACTOR * min_size;
        let config_horizon_size = params.horizon_size_bytes as i64;
        if (0..=8 * 1024 * 1024).contains(&config_horizon_size) {
            horizon_size = config_horizon_size as i32;
        }
        Self::new(min_size, horizon_size)
    }
}
